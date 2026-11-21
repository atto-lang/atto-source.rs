//! Canonical int/int -> float and fail-closed narrowing admission.
//! These Rust tests compile actual .atto source to Wasm artifacts but do not
//! substitute for executed WebAssembly guests (covered by Node suites).
use atto_compiler::compile;

#[test]
fn signed_int_division_has_float_result_and_wasm_artifact() {
    for body in [
        "fnc half(int a, int b): float { return a / b; } pub fnc main(): int { if half(5, 2) eq 2.5 then return 42; fi return 1; }",
        "fnc half(int a, int b): float { return a / b; } pub fnc main(): int { if half(-5, 2) eq -2.5 then return 42; fi return 1; }",
        "fnc half(int a, int b): float { return a / b; } pub fnc main(): int { if half(5, -2) eq -2.5 then return 42; fi return 1; }",
    ] {
        let guest = compile(body).unwrap_or_else(|e| panic!("division: {e}"));
        assert_eq!(&guest[..8], b"\\0asm\\x01\\0\\0\\0");
    }
}

#[test]
fn division_cannot_implicitly_narrow_to_integer_return_or_binding() {
    for source in [
        "pub fnc main(): int { return 5 / 2; }",
        "pub fnc main(): int { val result: int = 5 / 2; return result; }",
    ] {
        let error = compile(source).expect_err("float -> int narrowing must not emit Wasm");
        assert_eq!(error.code, "E_TYPE", "{source}: {error}");
    }
}
