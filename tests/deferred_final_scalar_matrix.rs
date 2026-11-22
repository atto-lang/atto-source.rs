//! Deferred-final scalar coverage complements the executed guest fixture corpus.
//! Compile tests alone cannot establish native/browser/independent backend parity.
use atto_compiler::compile;

fn compiled(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("expected Wasm guest: {e}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(source).unwrap(), "guest bytes must be deterministic");
}

#[test]
fn website_account_uses_deferred_final_identity() {
    let source = include_str!("playground_site_examples/classes.atto");
    assert!(source.contains("final int id;"));
    assert!(!source.contains("int id: 0"));
    compiled(source);
}

#[test]
fn supported_scalar_deferred_final_fields_compile_to_wasm() {
    for source in [
        include_str!("fixtures/pass/deferred-final-bool.atto"),
        include_str!("fixtures/pass/deferred-final-float.atto"),
        include_str!("fixtures/pass/deferred-final-string.atto"),
        include_str!("fixtures/pass/deferred-final-enum.atto"),
    ] {
        compiled(source);
    }
}

#[test]
fn deferred_final_scalar_invalid_paths_reject_before_wasm() {
    for (source, expected_code) in [
        (include_str!("fixtures/fail/deferred-final-bool-partial.atto"), "E_INIT"),
        (include_str!("fixtures/fail/deferred-final-float-premature-read.atto"), "E_INIT_READ"),
        (include_str!("fixtures/fail/deferred-final-string-double-write.atto"), "E_INIT_ONCE"),
        (include_str!("fixtures/fail/deferred-final-enum-preinitialized.atto"), "E_IMMUTABLE"),
    ] {
        let error = compile(source).expect_err("invalid constructor must not produce Wasm");
        assert_eq!(error.code, expected_code, "{error}");
    }
}
