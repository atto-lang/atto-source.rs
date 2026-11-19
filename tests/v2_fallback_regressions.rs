//! Regression gates for the experimental v2 compiler fallback.
//! These tests compile actual Atto source; parser acceptance alone is insufficient.
//! Until this branch has executed CI, these are authored tests, not a PASS claim.

fn v2_diagnostic(source: &str) -> atto_compiler::Diagnostic {
    let tokens = atto_compiler::lexer::lex(source).expect("fixture should lex");
    atto_compiler::v2::compile(tokens)
        .expect_err("invalid Atto source must never emit a v2 Wasm artifact")
}

#[test]
fn untyped_option_constructor_cannot_bypass_legacy_type_rejection() {
    let source = "pub fnc main(): int { val x = Some(42); return 0; }";
    let v2 = v2_diagnostic(source);
    assert_eq!(v2.code, "E_TYPE", "v2 must reject untyped Some without producing Wasm");
    let public = atto_compiler::compile(source).expect_err("fallback accepted invalid ADT source");
    assert_eq!(public.code, "E_TYPE");
}

#[test]
fn excessively_long_expression_chains_remain_nesting_errors() {
    for operator in [" + ", " * ", " - ", " == ", " < "] {
        let expression = vec!["1"; 3_000].join(operator);
        let source = format!("pub fnc main(): int {{ return {expression}; }}");
        let v2 = v2_diagnostic(&source);
        assert_eq!(v2.code, "E_NESTING", "v2 changed diagnostic for {operator:?}");
        let public = atto_compiler::compile(&source).expect_err("overlong expression compiled");
        assert_eq!(public.code, "E_NESTING", "fallback diagnostic changed for {operator:?}");
    }
}

#[test]
fn excessive_elif_chain_remains_nesting_error() {
    let arms = (1..300).map(|i| format!("elif x eq {i} then return {i};"))
        .collect::<Vec<_>>().join("\n");
    let source = format!(
        "pub fnc main(): int {{\nval x = 1;\nif x eq 0 then return 0;\n{arms}\nfi\nreturn 0;\n}}"
    );
    let v2 = v2_diagnostic(&source);
    assert_eq!(v2.code, "E_NESTING");
    let public = atto_compiler::compile(&source).expect_err("overlong elif compiled");
    assert_eq!(public.code, "E_NESTING");
}
