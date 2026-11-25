//! Adversarial edition-boundary checks: commas inside expressions and nested
//! control flow must never be mistaken for outer callable-body separators.
use atto_compiler::{
    compile,compile_with_version,lexer::lex,
    parser::{LanguageVersion,Parser},
    target_syntax::parse_target_declarations_versioned,
};

fn compile_both(src: &str) {
    let legacy=compile_with_version(src,LanguageVersion::Legacy)
        .unwrap_or_else(|e|panic!("legacy rejected valid source: {src}: {e}"));
    let strict=compile_with_version(src,LanguageVersion::Explicit)
        .unwrap_or_else(|e|panic!("explicit rejected valid source: {src}: {e}"));
    assert_eq!(legacy,strict,"common syntax must emit byte-for-byte identical Wasm");
    assert_eq!(legacy,compile(src).unwrap());
}

#[test]
fn comma_return_sequence_is_not_outer_body_mode() {
    for f in [
        "fnc f() -> int return 1, 42; end",
        "fnc f() -> int ^ 1, 42; end",
        "fnc f() -> int return seq 1, 42; end",
        "fnc f() -> int ^ seq 1, 42; end",
        "fnc f() -> int return seq(1, 42); end",
    ] {
        compile_both(&format!("{f} pub fnc main(): int {{ return f(); }}"));
    }
}

#[test]
fn parenthesized_seq_commas_in_statement_initializers_are_not_outer_body_mode() {
    compile_both("fnc f() -> int const int n = seq(1, 42); return n; end pub fnc main(): int { return f(); }");
}

#[test]
fn conditional_commas_do_not_reselect_the_body_mode() {
    let body="fnc f() -> int if true then ^ 1, 42; else ^ 0; fi end pub fnc main(): int { return f(); }";
    compile_both(body);
}

#[test]
fn target_mtd_preflight_uses_the_same_edition_for_nested_classes() {
    let f="define class C as pub get() -> int const int n = 40, n + 2 end end pub mtd (C self) bonus() -> int :: 0 end pub fnc main(): int { val c = new C(); return c.get(); }";
    let toks=lex(f).unwrap();
    assert!(parse_target_declarations_versioned(&toks,LanguageVersion::Legacy).is_ok());
    let d=parse_target_declarations_versioned(&toks,LanguageVersion::Explicit).unwrap_err();
    assert_eq!(d.code,"E_BODY_VERSION");
    let d=compile_with_version(f,LanguageVersion::Explicit).unwrap_err();
    assert_eq!(d.code,"E_BODY_VERSION");
}

#[test]
fn target_mtd_explicit_return_commas_stay_inside_the_return() {
    let f="define class C as final int seed: 40; end pub mtd (C self) get() -> int return 1, self.seed + 2; end pub fnc main(): int { val c = new C(); return c.get(); }";
    compile_both(f);
}

#[test]
fn target_mtd_parenthesized_seq_initializer_commas_stay_inside_the_expression() {
    let f="define class C as final int seed: 40; end pub mtd (C self) get() -> int const int n = seq(1, 42); return n; end pub fnc main(): int { val c = new C(); return c.get(); }";
    compile_both(f);
}

#[test]
fn version_diagnostic_is_not_used_to_launder_malformed_nested_sequences() {
    for src in [
        "fnc f() -> int return seq(1,,42); end pub fnc main(): int { return f(); }",
        "fnc f() -> int return seq(1;42); end pub fnc main(): int { return f(); }",
        "fnc f() -> int ^ 1,,42; end pub fnc main(): int { return f(); }",
    ] {
        let err=compile_with_version(src,LanguageVersion::Explicit).unwrap_err();
        assert_ne!(err.code,"E_BODY_VERSION", "actual malformed expression hidden by edition migration: {err}");
    }
}

#[test]
fn class_definition_and_default_parser_agree_on_legacy_semantics() {
    let src="define class C as pub answer() -> int const int n = 40, n + 2 end end pub fnc main(): int { val c = new C(); return c.answer(); }";
    let p=Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(p.classes.len(),1);
    assert!(compile(src).is_ok());
    assert_eq!(compile_with_version(src,LanguageVersion::Explicit).unwrap_err().code,"E_BODY_VERSION");
}

#[test]
fn bare_seq_initializer_is_not_falsely_advertised_as_supported() {
    let src="fnc f() -> int const int n = seq 1, 42; return n; end pub fnc main(): int { return f(); }";
    for version in [LanguageVersion::Legacy,LanguageVersion::Explicit] {
        let d=compile_with_version(src,version).unwrap_err();
        assert_eq!(d.code,"E_SYNTAX");
        assert_ne!(d.code,"E_BODY_VERSION");
    }
}
