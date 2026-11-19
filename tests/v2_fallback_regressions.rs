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


#[test]
fn v2_fallback_never_reinterprets_legacy_negative_fixtures_without_async_opt_in() {
    // These cases were proven to regress when the whole v2 pipeline was
    // invoked after every legacy rejection. Preserve both fail-closed behavior
    // and the legacy diagnostics rather than merely asserting an error exists.
    for name in [
        "106-propagate-wrong-return-family",
        "107-propagate-unwrapped-type",
        "108-propagate-from-non-adt-main",
        "109-deeper-option-nesting-unimplemented",
        "38-for-of",
        "39-end-form-body",
        "40-val-mut",
        "56-actor-calls-file-function",
        "60-pub-receive",
    ] {
        let path = format!("tests/fixtures/fail/{name}.atto");
        let source = std::fs::read_to_string(&path).expect("legacy rejection fixture");
        let toks = atto_compiler::lexer::lex(&source).expect("fixture should lex");
        let legacy = atto_compiler::parser::Parser::new(toks)
            .parse_program()
            .and_then(|program| atto_compiler::check::check(&program))
            .expect_err("the original frontend must reject this source");
        let public = atto_compiler::compile(&source)
            .expect_err("v2 fallback made previously invalid source executable");
        assert_eq!(public.code, legacy.code, "{name}: must preserve legacy diagnostic");
    }
}

#[test]
fn v2_opt_in_requires_real_suspends_declaration_not_comment_or_string() {
    let disguised = r#"// suspends fnc phantom(): int { return 42; }
pub fnc main(): int {
    stdio.println("suspends fnc");
    val mut x = 1;
    return x;
}
"#;
    let toks = atto_compiler::lexer::lex(disguised).expect("must tokenize");
    let legacy = atto_compiler::parser::Parser::new(toks)
        .parse_program()
        .and_then(|program| atto_compiler::check::check(&program))
        .expect_err("invalid legacy source");
    let public = atto_compiler::compile(disguised).expect_err("comments/strings enabled v2");
    assert_eq!(public.code, legacy.code);
}

#[test]
fn explicit_suspends_declaration_reaches_real_v2_compiler() {
    let source = r#"pub suspends fnc main(): int {
    val future: Future<int> = task { return 42; };
    return await future;
}
"#;
    let wasm = atto_compiler::compile(source).expect("explicit async signature must compile");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0", "emit a real guest Wasm module");
}
