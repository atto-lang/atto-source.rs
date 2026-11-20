//! Regression for the Oreslang #37/#38 arithmetic contract disagreement.
//! AttoLang's ordinary int expressions are checked at *runtime*; only
//! explicit compile-time-constant contexts classify provable overflow early.
//! The full scalar-v4 execution matrix lives in PR #399; do not duplicate it.

use atto_compiler::compile;

fn guest(body: &str) -> String {
    format!("pub fnc main(): int {{ {body} }}")
}

fn assert_const_overflow(source: &str) {
    let err = compile(source).expect_err("constant overflow must not emit guest Wasm");
    assert_eq!(err.code, "E_CONST_OVERFLOW", "{source}: {err}");
}

fn assert_compiles(source: &str) {
    let wasm = compile(source).unwrap_or_else(|d| panic!("{source}: {d}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{source}: no guest Wasm");
}

#[test]
fn explicit_const_overflow_is_compile_time_error() {
    for body in [
        "const value = 9223372036854775807 + 1; return 0;",
        "const int value = -9223372036854775808 - 1; return 0;",
        "const value: int = 3037000500 * 3037000500; return 0;",
        // Evaluating a parent must never suppress an overflowing child.
        "const value = (9223372036854775807 + 1) * 0; return 0;",
        // Named Boolean builtins evaluate both arguments, unlike ternary.
        "const value = And(false, 9223372036854775807 + 1 eq 0); return 0;",
        "const value = Or(true, 9223372036854775807 + 1 eq 0); return 0;",
        "const max = 9223372036854775807; const value = max + 1; return 0;",
        "const min = -9223372036854775808; const value = -min; return 0;",
        "const flag = true; const value = flag ? 9223372036854775807 + 1 : 0; return 0;",
    ] {
        assert_const_overflow(&guest(body));
    }
}

#[test]
fn ordinary_arithmetic_stays_checked_at_runtime_not_rejected_early() {
    for body in [
        "return 9223372036854775807 + 1;",
        "val value = 9223372036854775807 + 1; return value;",
        "let value = 3037000500 * 3037000500; return value;",
        "val max = 9223372036854775807; return max + 1;",
    ] {
        assert_compiles(&guest(body));
    }
    // A call with runtime-dependent input is never rejected by static
    // arithmetic classification, but its Wasm body still has checked ops.
    assert_compiles(
        "fnc add(a: int, b: int): int { return a + b; } \
         pub fnc main(): int { return add(9223372036854775807, 1); }",
    );
}

#[test]
fn class_const_is_distinct_from_final_and_val_fields() {
    let class = |modifier: &str| {
        format!(
            "define class Limits as pub {modifier} int maximum = 9223372036854775807 + 1; end \
         pub fnc main(): int {{ return 0; }}"
        )
    };
    assert_const_overflow(&class("const"));
    // The non-const initializer remains emitted to the checked guest path
    // instead of being rejected as if it were a compile-time constant.
    assert_compiles(&class("final"));
}

#[test]
fn constant_conditionals_evaluate_only_the_selected_arm() {
    assert_compiles(&guest(
        "const flag = false; const value = flag ? 9223372036854775807 + 1 : 42; \
         return value;",
    ));
    assert_compiles(&guest("const value = true ? 42 : 3037000500 * 3037000500; return value;"));
}

#[test]
fn signed_minimum_literal_and_safe_constant_boundaries_still_compile() {
    assert_compiles(&guest(
        "const min = -9223372036854775808; const minus_one = min + 1; \
         const maximum = 9223372036854775807; return (maximum - maximum) + 42;",
    ));
}

#[test]
fn type_errors_are_not_masked_by_constant_arithmetic_classification() {
    let error = compile(&guest("const bad = true + 1; return 0;")).unwrap_err();
    assert_eq!(error.code, "E_TYPE");
}
