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

#[test]
fn zero_payload_is_not_missing_or_error() {
    let source = include_str!("core_option_result/tag_zero_42.atto");
    let guest = compile(source)
        .unwrap_or_else(|error| panic!("tag-zero must emit real wasm: {error}"));
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"), "real Wasm required");
    assert_eq!(guest, compile(source).unwrap(), "deterministic output");
}

#[test]
fn missing_none_arm_is_nonexhaustive() {
    let source = include_str!("core_option_result/negative_nonexhaustive.atto");
    let error = compile(source).expect_err("cannot admit incomplete Option match");
    assert_eq!(error.code, "E_MATCH", "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
}
