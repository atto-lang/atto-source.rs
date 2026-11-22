//! ActorFuture<int> v1: actor-bound single-reply manifest and ownership.
use atto_compiler::{compile,compile_actor_future_int_aot,compile_future_int_aot};

const SOURCE:&str="pub susp fnc main():int { \
 val first:ActorFuture<int>=actor_future_int(7,41); \
 await first; \
 val second:ActorFuture<int>=actor_future_int(7,42); \
 return await second; }";

fn find(wasm:&[u8],pattern:&str)->bool {
    wasm.windows(pattern.len()).any(|p|p==pattern.as_bytes())
}
#[test]
fn actor_future_emits_versioned_host_binding_without_guest_jit(){
    let a=compile_actor_future_int_aot(SOURCE).expect("linear ActorFuture<int> must compile");
    assert_eq!(a,compile_actor_future_int_aot(SOURCE).unwrap(),"AOT determinism");
    assert!(find(&a,"atto.browser.source.actor_future.v1"));
    assert!(find(&a,"atto.browser.guest.v5"));
    assert!(!find(&a,"atto.browser.source.future.v1"));
    for export in ["task_pending_v5","task_complete_v5","task_resume_v5","task_cancel_v5"] {
        assert!(find(&a,export),"{export}");
    }
    assert!(compile(SOURCE).is_err(),"normal compile never reinterprets actor replies");
    assert!(compile_future_int_aot(SOURCE).is_err(),"Future<int> target must not steal ActorFuture");
}
#[test]
fn actor_future_linear_ownership_and_streaming_fail_closed(){
    for source in [
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); await f; return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await actor_future_int(7,42); }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); val g:int=f; return await f; }",
        "pub susp fnc main():int { let f:ActorFuture<int>=actor_future_int(7,41); return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await 41; }",
        "pub susp fnc main():int { val f:ActorFuture<bool>=actor_future_int(7,41); return await f; }",
        "pub susp fnc main():int { val f:ActorStream<int>=actor_future_int(7,41); return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,true); return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(0,41); return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,-1); return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await actor_future_int(0,42); }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); if true then return await f; fi return 1; }",
    ] {
        assert!(compile_actor_future_int_aot(source).is_err(),"must reject unsupported actor future: {source}");
    }
}
#[test]
fn actor_future_requires_static_actor_identity_and_request_key(){
    for source in [
        "pub susp fnc main():int { val actor:int=7; val f:ActorFuture<int>=actor_future_int(actor,41); return await f; }",
        "pub susp fnc main():int { val key:int=41; val f:ActorFuture<int>=actor_future_int(7,key); return await f; }",
        "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41+1); return await f; }",
    ]{
        assert!(compile_actor_future_int_aot(source).is_err(),"dynamic actor/key not yet supported: {source}");
    }
}

#[test]
fn explicit_take_actor_future_is_one_shot_and_preserves_existing_aot_abi(){
    let src="pub susp fnc main():int { \
      val a:ActorFuture<int>=actor_future_int(7,41); \
      await take! a; \
      val b:ActorFuture<int>=actor_future_int(7,42); \
      return await take! b; }";
    let wasm=compile_actor_future_int_aot(src).expect("named owned take! actor receipt");
    let again=compile_actor_future_int_aot(src).expect("deterministic guest");
    assert_eq!(wasm,again);
    assert!(find(&wasm,"atto.browser.source.actor_future.v1"));
    assert!(find(&wasm,"atto.browser.guest.v5"));
    assert!(!find(&wasm,"atto.browser.source.future.v1"));
    // A different surface must not silently assume the actor reply profile.
    assert!(compile(src).is_err());
    assert!(compile_future_int_aot(src).is_err());
}

#[test]
fn explicit_take_actor_future_rejects_alias_forgery_replay_and_bad_lexing(){
    let bad=[
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); await take! f; return await f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); await f; return await take! f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take! actor_future_int(7,41); }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take! 41; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await borrow! f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take! borrow! f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take ! f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take/*comment*/! f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take\n! f; }",
      "pub susp fnc main():int { val f:ActorFuture<int>=actor_future_int(7,41); return await take! nonexistent; }",
    ];
    for src in bad {
        assert!(compile_actor_future_int_aot(src).is_err(),
            "must fail closed, never emit AOT for invalid take!: {src}");
    }
}

#[test]
fn generic_native_await_adapter_formals_do_not_downgrade_to_int_aot(){
    for src in [
      "pub susp fnc await_actor<T>(take! ActorFuture<T> reply):T { return await reply; }",
      "pub susp fnc await_actor_result<T,E>(take! ActorFuture<Result<T,E>> reply):Result<T,E> { return await reply; }",
      "pub susp fnc main():int { val f:ActorFuture<bool>=actor_future_int(7,41); return await take! f; }",
    ]{
        assert!(compile_actor_future_int_aot(src).is_err(),
            "generic typed adapters require future monomorphization and verified result ABI: {src}");
    }
}
