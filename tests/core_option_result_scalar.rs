//! Compile real .atto scalar sum fixtures through the Rust compiler.
//! Generic source modules are a separate, deliberately unverified gate:
//! these two tests demonstrate the nominal substrate, not installed libs.
use atto_compiler::compile;

#[test]
fn scalar_option_result_guest_emits_deterministic_wasm() {
    let source = include_str!("core_option_result/positive_42.atto");
    let first = compile(source)
        .unwrap_or_else(|error| panic!("expected executable wasm, got {error}"));
    assert!(first.starts_with(b"\0asm\x01\0\0\0"), "must emit real Wasm");
    assert_eq!(first, compile(source).unwrap(), "emission must be deterministic");
}

#[test]
fn wrong_nominal_carrier_never_emits_artifact() {
    let source = include_str!("core_option_result/negative_wrong_carrier.atto");
    let error = compile(source).expect_err("Err cannot construct Option<int>");
    assert_eq!(error.code, "E_TYPE", "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}
