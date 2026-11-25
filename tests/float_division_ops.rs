//! Checked int/int -> int; explicit float operands retain floating division.
//! Compiles source to Wasm; actual guest execution is tested by Node suites.
use atto_compiler::compile;

#[test]
fn signed_int_division_has_int_result_and_wasm_artifact() {
    for source in [
        "pub fnc main(): int { return 5 / 2; }",
        "pub fnc main(): int { return -5 / 2; }",
        "pub fnc main(): int { val result: int = 5 / 2; return result; }",
        "fnc quotient(int a, int b): int { return a / b; } pub fnc main(): int { return quotient(-7, 3); }",
        "fnc half(float a, float b): float { return a / b; } pub fnc main(): int { if half(5.0, 2.0) eq 2.5 then return 42; fi return 1; }",
    ] {
        let guest = compile(source).unwrap_or_else(|e| panic!("division: {e}"));
        assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    }
}

#[test]
fn explicit_float_division_cannot_implicitly_narrow() {
    for source in [
        "pub fnc main(): int { return 5.0 / 2.0; }",
        "pub fnc main(): int { val result: int = 5.0 / 2.0; return result; }",
        "pub fnc main(): int { return true / 2; }",
    ] {
        let error = compile(source).expect_err("invalid operand or narrowing must not emit Wasm");
        assert_eq!(error.code, "E_TYPE", "{source}: {error}");
    }
}
