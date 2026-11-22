//! Opt-in v5 AOT externally pending scalar future source admission.
use atto_compiler::{compile,compile_portable_v4,compile_portable_v5};

fn has_marker(wasm:&[u8],wanted:&str,payload:u8)->bool {
    let pattern=[wanted.as_bytes(),&[payload]].concat();
    wasm.windows(pattern.len()).any(|x|x==pattern)
}
#[test]
fn portable_v5_emits_deterministic_versioned_pending_future_exports() {
    let src="pub susp fnc main(): int { val key: int=41; await key; return await 42; }";
    let bytes=compile_portable_v5(src).expect("typed future-key scalar source must compile");
    assert_eq!(bytes,compile_portable_v5(src).unwrap());
    assert!(has_marker(&bytes,"atto.browser.guest.v5",5));
    assert!(!has_marker(&bytes,"atto.browser.guest.v4",4));
    assert!(!has_marker(&bytes,"atto.browser.guest.v3",3));
    for name in ["task_start_v5","task_resume_v5","task_cancel_v5",
        "task_drop_v5","task_result_v5","task_capacity_v5",
        "task_complete_v5","task_pending_v5"] {
        assert!(bytes.windows(name.len()).any(|w|w==name.as_bytes()),"{name}");
    }
    let old=compile(src).unwrap();
    assert!(has_marker(&old,"atto.browser.guest.v3",3));
    assert!(has_marker(&compile_portable_v4(src).unwrap(),
        "atto.browser.guest.v4",4));
}
#[test]
fn portable_v5_rejects_unsupported_async_shapes_instead_of_fallback() {
    for src in [
        "pub susp fnc main(): int { val f=task {return 42;}; return await f; }",
        "pub susp fnc main(): int { if true then return await 41; fi return 42; }",
        "pub susp fnc main(): int { val x: int=40+2; return await x; }",
        "pub susp fnc main(): int { await 41; return 42; }",
        "pub susp fnc main(): int { val b: bool=true; return await 42; }",
    ] {
        assert!(compile_portable_v5(src).is_err(),"{src}");
    }
}
