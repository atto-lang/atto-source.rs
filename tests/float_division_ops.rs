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

#[test]
fn named_integer_operations_emit_wasm_and_reject_invalid_arguments() {
    for source in [
        "pub fnc main(): int { return Integer.rem(-7, 3); }",
        "pub fnc main(): int { return Integer.mod(-7, -3); }",
        "pub fnc main(): int { return Integer.rem(-9223372036854775808, -1); }",
        "pub fnc main(): int { return Integer.mod(-1, -9223372036854775808); }",
    ] {
        let guest = compile(source).unwrap_or_else(|e| panic!("named integer operation: {e}"));
        assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    }
    for source in [
        "pub fnc main(): int { return Integer.rem(true, 3); }",
        "pub fnc main(): int { return Integer.mod(7); }",
        "pub fnc main(): int { return Integer.rem(7, 3, 1); }",
        "pub fnc main(): int { return Integer.unknown(7, 3); }",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn mixed_integer_float_ordering_cannot_silently_round_i64() {
    for op in ["<", "<=", ">", ">="] {
        for (a, b) in [("9007199254740993", "9007199254740992.0"),
                       ("9007199254740992.0", "9007199254740993")] {
            let source = format!("pub fnc main(): int {{ if {a} {op} {b} then return 42; fi return 1; }}");
            let error = compile(&source).expect_err("unproved mixed comparison must not emit Wasm");
            assert_eq!(error.code, "E_TYPE", "{source}: {error}");
        }
    }
    for source in [
        "pub fnc main(): int { if 9007199254740993 > 9007199254740992 then return 42; fi return 1; }",
        "pub fnc main(): int { if 2.5 > 2.0 then return 42; fi return 1; }",
    ] {
        let guest = compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    }
}
