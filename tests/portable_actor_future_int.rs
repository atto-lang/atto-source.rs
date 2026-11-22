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
