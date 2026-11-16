//! Exact public /play match-types-is preview uses monomorphic checked Any.
//! The guest executes the original runtime integer comparison and guard.
use atto_compiler::compile;
const SITE: &str = include_str!("playground_previews/match-types-is.atto");
fn good(source: &str) -> Vec<u8> {
    let wasm = compile(source).unwrap_or_else(|d| panic!("source must compile: {d}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).unwrap(), "deterministic guest");
    wasm
}
fn bad(source: &str, wanted: &str) {
    let d=compile(source).expect_err("unsupported dynamic Any must emit no guest");
    assert_eq!(d.code,wanted,"{d}");
}
#[test]
fn exact_published_type_test_source_is_compiled_to_real_guest() {
    let guest=good(SITE);
    assert!(guest.windows(b"positive integer".len())
        .any(|w|w==b"positive integer"),"live value guard must still emit string branch");
}
#[test]
fn guard_is_not_replaced_with_a_constant_result() {
    // Change the checked, immutable source literal while keeping the same
    // single concrete int type. The guest must choose its other-int branch.
    good(&SITE.replace("val value: Any = 42;", "val value: Any = 0;")
        .replace("value == 42", "value == 0"));
}
#[test]
fn dynamic_any_and_public_any_api_fail_closed() {
    bad(&SITE.replace("val value: Any = 42;", "let value: Any = 42;"), "E_UNSUPPORTED_TYPE");
    bad(&SITE.replace("val value: Any = 42;", "val value: Any = get_value();"), "E_UNSUPPORTED_TYPE");
    bad(&SITE.replace("fnc describe(Any value)", "pub fnc describe(Any value)"), "E_UNSUPPORTED");
    bad(&SITE.replace("val value: Any = 42;", "val value: Any = true;"), "E_UNSUPPORTED_TYPE");
}
#[test]
fn cannot_pass_unproven_or_wrong_dynamic_payload() {
    bad(&SITE.replace("describe(value)", "describe(unknown)"), "E_UNSUPPORTED");
    bad(&SITE.replace("describe(value)", "describe(1)"), "E_UNSUPPORTED");
}
