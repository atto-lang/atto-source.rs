//! Type admission is not a guest allocation proof.
use atto_compiler::{compile, compile_js};

#[test]
fn int_factory_operands_are_checked_in_both_executable_targets() {
    for src in [
        "fnc main(): int { let x = Set.of(1, 2, 1); return 0; }",
        "fnc main(): int { let n = 7; let x = Set.of(n, n + 1); return 0; }",
    ] {
        let wasm = compile(src).expect_err("unimplemented storage");
        let js = compile_js(src).expect_err("unimplemented storage");
        assert_eq!(wasm.code, "E_UNSUPPORTED");
        assert_eq!(wasm.message, js.message);
        assert!(wasm.message.contains("typed Set<int>.of"), "{wasm}");
    }
}

#[test]
fn invalid_operands_and_empty_inference_do_not_admit_storage() {
    for (expr, expected) in [
        ("Set.of(missing)", "E_UNBOUND"),
        ("Set.of(1, true)", "E_UNSUPPORTED"),
        ("Set.of()", "E_UNSUPPORTED"),
        ("Set.of(1, 2 + true)", "E_TYPE"),
    ] {
        let src = format!("fnc main(): int {{ let x = {expr}; return 0; }}");
        let wasm = compile(&src).expect_err(expr);
        let js = compile_js(&src).expect_err(expr);
        assert_eq!(wasm.code, expected, "{expr}: {wasm}");
        assert_eq!(wasm.code, js.code);
    }
}

#[test]
fn local_shadow_does_not_authorize_builtin_set_factory() {
    let src = "fnc main(): int { let Set = 5; let x = Set.of(1); return 0; }";
    let err = compile(src).expect_err("local is not a collection factory");
    assert!(!err.message.contains("typed Set<int>.of"), "{err}");
}
