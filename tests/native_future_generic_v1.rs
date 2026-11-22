//! Canonical source-level Future<T> identity adapter specialization,
//! with exact move semantics and no runtime generic interpreter.
use atto_compiler::{compile,compile_future_int_aot,compile_actor_future_int_aot};

const FUTURE:&str=r#"
pub susp fnc await_future<T>(take! Future<T> receipt) -> T {
  return await receipt;
}
pub susp fnc main():int {
  val f:Future<int>=pending_future_int(41);
  return await await_future<int>(take! f);
}
"#;
const ACTOR:&str=r#"
pub susp fnc await_actor<T>(take! ActorFuture<T> reply) -> T {
  return await reply;
}
pub susp fnc main():int {
  val f:ActorFuture<int>=actor_future_int(7,41);
  return await await_actor<int>(take! f);
}
"#;
fn contains(bytes:&[u8], marker:&str)->bool{
    bytes.windows(marker.len()).any(|part|part==marker.as_bytes())
}
#[test]
fn compiles_source_declared_future_int_adapter_to_real_aot_await(){
    let wasm=compile_future_int_aot(FUTURE).expect("generic T=int identity adapter must specialize");
    assert_eq!(wasm,compile_future_int_aot(FUTURE).unwrap());
    assert!(contains(&wasm,"atto.browser.source.future.v1"));
    assert!(contains(&wasm,"atto.browser.guest.v5"));
    assert!(compile(FUTURE).is_err(),"default compile may not steal typed Future AOT");
}
#[test]
fn compiles_source_declared_actor_future_int_adapter_without_forging_actor(){
    let wasm=compile_actor_future_int_aot(ACTOR).expect("generic T=int actor identity adapter must specialize");
    assert_eq!(wasm,compile_actor_future_int_aot(ACTOR).unwrap());
    assert!(contains(&wasm,"atto.browser.source.actor_future.v1"));
    assert!(contains(&wasm,"atto.browser.guest.v5"));
    assert!(!contains(&wasm,"atto.browser.source.future.v1"));
    assert!(compile_future_int_aot(ACTOR).is_err());
}
#[test]
fn generic_future_call_requires_unique_owned_take_and_single_await(){
    let parts=[
      "return await await_future<int>(f);",
      "return await await_future<int>(borrow! f);",
      "return await await_future<int>(copy! f);",
      "return await await_future<int>(take! 42);",
      "return await await_future<int>(take! other);",
      "return await await_future<bool>(take! f);",
      "return await await_future<Future<int>>(take! f);",
      "return await await_future<int>(take ! f);",
      "return await await_future<int>(take/*x*/! f);",
      "return await await_future<int>(take\n! f);",
      "return await_future<int>(take! f);",
      "return await await_actor<int>(take! f);",
    ];
    for wrong in parts{
        let candidate=FUTURE.replace("return await await_future<int>(take! f);",wrong);
        assert!(compile_future_int_aot(&candidate).is_err(),
            "must reject unsupported or forged generic call: {wrong}");
    }
    let reused=FUTURE.replace(
        "return await await_future<int>(take! f);",
        "await await_future<int>(take! f); return await f;");
    let e=compile_future_int_aot(&reused).expect_err("double await must not emit AOT");
    assert_eq!(e.code,"E_MOVED");
}
#[test]
fn generic_adapter_definition_is_sealed_and_cannot_reinterpret_body(){
    let bad=[
      FUTURE.replace("take! Future<T> receipt","Future<T> receipt"),
      FUTURE.replace("return await receipt;","return 42;"),
      FUTURE.replace("return await receipt;","await receipt; return 42;"),
      FUTURE.replace("Future<T> receipt","ActorFuture<T> receipt"),
      FUTURE.replace(" -> T {"," -> int {"),
      FUTURE.replace("take! Future<T>","take ! Future<T>"),
      FUTURE.replace("take! Future<T>","take/*x*/! Future<T>"),
      FUTURE.replace("return await receipt;","return await take! receipt;"),
      FUTURE.replace("pub susp fnc","pub fnc"),
      FUTURE.replace("T> receipt) -> T","T> receipt) -> bool"),
      FUTURE.replace("pub susp fnc main():int", "pub susp fnc await_future<T>(take! Future<T> receipt) -> T { return await receipt; }\npub susp fnc main():int"),
    ];
    for candidate in bad {
        assert!(compile_future_int_aot(&candidate).is_err(),
            "unverified generic declaration must not emit an AOT artifact");
    }
}
#[test]
fn actor_generic_replay_cross_profile_and_result_shape_reject(){
    let bad=[
      ACTOR.replace("return await await_actor<int>(take! f);","await await_actor<int>(take! f); return await f;"),
      ACTOR.replace("return await await_actor<int>(take! f);","return await await_future<int>(take! f);"),
      ACTOR.replace("return await await_actor<int>(take! f);","return await await_actor<Result<int,string>>(take! f);"),
      ACTOR.replace("return await await_actor<int>(take! f);","return await await_actor<int>(take! actor_future_int(7,41));"),
      ACTOR.replace("take! ActorFuture<T> reply","ActorFuture<T> reply"),
      ACTOR.replace("actor_future_int(7,41)","actor_future_int(7,true)"),
    ];
    for candidate in bad {
        assert!(compile_actor_future_int_aot(&candidate).is_err(),
            "ActorFuture provenance/generic result must remain fail-closed");
    }
}
