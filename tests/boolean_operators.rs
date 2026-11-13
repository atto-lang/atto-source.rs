//! Canonical &&, || and ^^ acceptance, type safety and diagnostics.
use atto_compiler::compile;

#[test]
fn all_canonical_boolean_forms_compile_in_expressions_and_conditions() {
    for expression in [
        "true && false", "false || true", "true ^^ false",
        "false || true && true", "true ^^ false && true",
        "true && false || true",
    ] {
        let source = format!("pub fnc main(): int {{ val b = {expression}; if b then return 42; fi return 0; }}");
        assert!(compile(&source).is_ok(), "failed to compile {expression}");
    }
}

#[test]
fn logical_operands_require_bool_and_never_coerce_int() {
    for expression in ["1 && true", "true || 1", "false ^^ 0", "1 && 2"] {
        let source = format!("pub fnc main(): int {{ val b = {expression}; return 0; }}");
        let d = compile(&source).expect_err(expression);
        assert_eq!(d.code, "E_TYPE", "{expression}: {d}");
        assert!(d.pos.line >= 1 && d.pos.col >= 1);
    }
}

#[test]
fn standalone_bitwise_tokens_and_incomplete_expressions_fail_closed() {
    for expression in ["true & false", "true | false", "true ^ false", "true &&", "false ||", "true ^^"] {
        let source = format!("pub fnc main(): int {{ val x = {expression}; return 0; }}");
        assert!(compile(&source).is_err(), "accepted malformed/legacy bitwise: {expression}");
    }
    assert!(compile("pub fnc main(): int { return 42; }").is_ok());
}
