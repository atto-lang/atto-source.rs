//! Typed, single-use Future<int> source admission lowered onto verified v5.
use atto_compiler::{compile,compile_portable_v5,compile_future_int_aot};

fn marker(bytes:&[u8],name:&str,val:u8)->bool {
    let pattern=[name.as_bytes(),&[val]].concat();
    bytes.windows(pattern.len()).any(|x|x==pattern)
}
const TYPED:&str="pub susp fnc main(): int { \
    val key: int=41; \
    val first: Future<int>=pending_future_int(key); \
    await first; \
    val second: Future<int>=pending_future_int(42); \
    return await second; }";
#[test]
fn bounded_owned_future_int_lowers_to_real_pending_v5_guest() {
    let wasm=compile_future_int_aot(TYPED).expect("typed linear futures must compile to AOT guest");
    assert_eq!(wasm,compile_future_int_aot(TYPED).unwrap(),"AOT deterministic");
    assert!(marker(&wasm,"atto.browser.source.future.v1",1));
    assert!(marker(&wasm,"atto.browser.guest.v5",5));
    assert!(!marker(&wasm,"atto.browser.guest.v4",4));
    for name in ["task_start_v5","task_pending_v5",
        "task_complete_v5","task_resume_v5","task_drop_v5"] {
        assert!(wasm.windows(name.len()).any(|w|w==name.as_bytes()),"{name}");
    }
    assert!(compile(TYPED).is_err(),"default target must not silently accept a v5-only Future constructor");
    assert!(compile_portable_v5(TYPED).is_err(),"legacy keyed v5 source target remains unchanged");
}
#[test]
fn rejects_future_use_after_move_and_unconsumed_future() {
    for src in [
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); await f; return await f; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); return await pending_future_int(42); }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); val g: int=f; return await f; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); val g: Future<int>=pending_future_int(f); return await g; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); return await 41; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); return f; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); return await pending_future_int(f); }",
    ] {
        assert!(compile_future_int_aot(src).is_err(),"future ownership failure must reject: {src}");
    }
}
#[test]
fn rejects_wrong_type_branches_mutation_and_unsupported_generic_future() {
    for src in [
        "pub susp fnc main():int { val f: Future<bool>=pending_future_int(41); return await f; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(true); return await f; }",
        "pub susp fnc main():int { let f: Future<int>=pending_future_int(41); return await f; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); if true then return await f; fi return 42; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); return await f + 1; }",
        "pub fnc main():int { val f: Future<int>=pending_future_int(41); return await f; }",
        "pub susp fnc main():int { val f=pending_future_int(41); return await f; }",
        "pub susp fnc main():int { val f: Future<int>=pending_future_int(41); return await pending_future_int(41) + 1; }",
    ] {
        assert!(compile_future_int_aot(src).is_err(),"fail-closed unsupported future expression: {src}");
    }
}
