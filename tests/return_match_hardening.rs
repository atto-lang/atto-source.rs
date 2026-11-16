//! Hard bounds and error recovery for the return-position match feature.
#[test]
fn return_match_arm_budget_fails_closed_and_compiler_recovers() {
    let arms: String = (0..300).map(|i| format!("on {i} -> {i},\n")).collect();
    let source = format!("pub fnc main(): int {{ return match 5 over\n{arms}default -> -1,\nend }}");
    let error = atto_compiler::compile(&source).expect_err("too many arms must be rejected");
    assert_eq!(error.code, "E_NESTING", "{error}");
    let ordinary = "pub fnc main(): int { return match 5 over on 5 -> 42, default -> 0, end }";
    assert!(atto_compiler::compile(ordinary).is_ok(), "compiler recovers after bounded input");
}

#[test]
fn missing_match_terminator_never_produces_a_binary() {
    let source = "pub fnc main(): int { return match 3 over on 3 -> 1, default -> 2,";
    let error = atto_compiler::compile(source).expect_err("missing end must be rejected");
    assert_eq!(error.code, "E_EOF", "{error}");
}
