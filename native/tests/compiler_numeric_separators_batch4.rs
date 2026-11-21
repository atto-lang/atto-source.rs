//! Regression tests for decimal digit separator admission in the Rust frontend.
//! These test the real source parser and the normal compile + effect-IR paths.
use oreslang_source_rs::{compile, compile_with_effect_ir, frontend, mir};

fn reject(source: &str, class: &str) {
    let diagnostic = frontend::parse(source).expect_err("malformed source was accepted");
    assert_eq!(diagnostic.class, class, "source: {source:?}");
    let compile_error = compile(source).expect_err("malformed source compiled");
    assert!(
        compile_error.starts_with(class),
        "wrong compile diagnostic for {source:?}: {compile_error}"
    );
    assert!(
        compile_with_effect_ir(source).is_err(),
        "paired compilation admitted malformed source: {source:?}"
    );
}

#[test]
fn rejects_trailing_underscore_in_number() {
    for literal in ["1_", "99_", "0_", "9223372036854775807_"] {
        reject(
            &format!("pub fnc main(): int {{ return {literal}; }}"),
            "syntax_error",
        );
    }
}

#[test]
fn rejects_doubled_underscore_in_number() {
    for literal in ["1__0", "9__223", "0__0", "9223372036854775__807"] {
        reject(
            &format!("pub fnc main(): int {{ return {literal}; }}"),
            "syntax_error",
        );
    }
}

#[test]
fn rejects_repeated_separators_in_negative_number() {
    for literal in ["-1__0", "-9__223", "-0_", "-9_223_372_036__854_775_808"] {
        reject(
            &format!("pub fnc main(): int {{ return {literal}; }}"),
            "syntax_error",
        );
    }
}

#[test]
fn underscore_error_offset_identifies_first_bad_separator() {
    let source = "pub fnc main(): int { return 12__34; }";
    let error = frontend::parse(source).unwrap_err();
    assert_eq!(error.class, "syntax_error");
    assert_eq!(error.offset, source.find("__").unwrap());
}

#[test]
fn negative_underscore_error_offset_identifies_bad_separator() {
    let source = "pub fnc main(): int { return -12__34; }";
    let error = frontend::parse(source).unwrap_err();
    assert_eq!(error.class, "syntax_error");
    assert_eq!(error.offset, source.find("__").unwrap());
}

#[test]
fn admits_single_underscores_between_digits() {
    let source = "pub fnc main(): int { return 9_007_199_254_740_993 - 9_007_199_254_740_992; }";
    assert!(!compile(source).unwrap().is_empty());
    let (_object, ir) = compile_with_effect_ir(source).unwrap();
    assert_eq!(ir["schema_version"], 1);
}

#[test]
fn admits_signed_i64_minimum_with_valid_separators() {
    let source = "pub fnc main(): int { return -9_223_372_036_854_775_808; }";
    let ast = frontend::parse(source).unwrap();
    assert_eq!(
        ast.functions[0].expression,
        frontend::Expr::Integer(i64::MIN)
    );
    assert!(!compile(source).unwrap().is_empty());
}

#[test]
fn admits_signed_i64_maximum_with_valid_separators() {
    let source = "pub fnc main(): int { return 9_223_372_036_854_775_807; }";
    let ast = frontend::parse(source).unwrap();
    assert_eq!(
        ast.functions[0].expression,
        frontend::Expr::Integer(i64::MAX)
    );
    assert!(!compile(source).unwrap().is_empty());
}

#[test]
fn parsed_ast_is_identical_with_and_without_valid_separators() {
    let plain = frontend::parse("fnc f(): int { return 9007199254740993; }").unwrap();
    let separated = frontend::parse("fnc f(): int { return 9_007_199_254_740_993; }").unwrap();
    assert_eq!(plain, separated);
}

#[test]
fn separated_numeric_local_initializer_is_validated_in_mir() {
    let source = "pub fnc main(): int { const x: int = 4_0; const y: int = x + 2; return y; }";
    let program = frontend::parse(source).unwrap();
    let module = mir::lower(program).unwrap();
    assert_eq!(module.functions[0].locals.len(), 2);
    assert!(!compile(source).unwrap().is_empty());
}

#[test]
fn malformed_numeric_local_initializer_fails_compilation() {
    reject(
        "pub fnc main(): int { const x: int = 4__0; return x; }",
        "syntax_error",
    );
}

#[test]
fn numeric_tokens_inside_nested_comments_do_not_create_functions() {
    let source = "/* 1__0 /* -9__0 */ ignored */ pub fnc main(): int { return 4_2; }";
    let parsed = frontend::parse(source).unwrap();
    assert_eq!(parsed.functions.len(), 1);
    assert_eq!(
        parsed.functions[0].expression,
        frontend::Expr::Integer(42)
    );
    assert!(!compile(source).unwrap().is_empty());
}
