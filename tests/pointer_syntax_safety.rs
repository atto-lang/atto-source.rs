//! Pointer syntax safety boundary for the restricted executable browser compiler.
//! These tests are negative admission gates, NOT proof of Pointer<T> lowering.

fn diagnostic(source: &str, expected: &str) {
    let err = atto_compiler::compile(source).expect_err("unsupported pointer syntax emitted Wasm");
    assert_eq!(err.code, expected, "{err}");
}

#[test]
fn c_style_unary_pointer_operators_are_rejected_without_an_artifact() {
    diagnostic(
        "pub fnc main(): int { val x = 2; return &x; }",
        "E_POINTER_SYNTAX",
    );
    diagnostic(
        "pub fnc main(): int { val x = 2; return *x; }",
        "E_POINTER_SYNTAX",
    );
    diagnostic(
        "pub fnc main(): int { val x = 2; return (*x); }",
        "E_POINTER_SYNTAX",
    );
}

#[test]
fn pointer_builtin_requires_real_typed_aot_lowering() {
    diagnostic(
        "pub fnc main(): int { val p: Pointer<int> = 0; return 0; }",
        "E_UNSUPPORTED_TYPE",
    );
    diagnostic(
        "pub fnc main(): Pointer<int> { return 0; }",
        "E_UNSUPPORTED_TYPE",
    );
    diagnostic(
        "define class Pointer as end\npub fnc main(): int { return 0; }",
        "E_RESERVED_TYPE",
    );
}

#[test]
fn ordinary_multiplication_still_compiles_as_wasm() {
    let bytes = atto_compiler::compile("pub fnc main(): int { return 6 * 7; }")
        .expect("binary * must remain multiplication");
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}
