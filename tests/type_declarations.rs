//! Rust regression matrix for proposed AttoLang type headers.
//! Syntax preflight is deliberately distinct from executable Wasm emission.
use atto_compiler::type_declarations::{
    contains_type_declaration, parse_source, TypeDeclarationKind, TypeExpression,
};
use atto_compiler::lexer::lex;

fn accepts(src: &str) {
    parse_source(src).unwrap_or_else(|d| panic!("{src:?}: {}", d.render()));
}
fn rejects(src: &str, code: &str) {
    let d = match parse_source(src) {
        Ok(_) => panic!("unexpectedly accepted: {src}"),
        Err(d) => d,
    };
    assert_eq!(d.code, code, "for {src}: {}", d.render());
}

#[test]
fn proposed_examples_are_accepted_by_type_preflight() {
    accepts("define type Foo as bool | int");
    accepts("declare type Foo = bool | int");
    accepts("define type Bar as interface Z {}");
    accepts("define type Bar as trait Z {}");
    accepts("declare type Star = Z;");
}

#[test]
fn declaration_and_definition_can_match_regardless_of_union_order() {
    let v = parse_source("declare type Foo = bool | int; define type Foo as int | bool").unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].kind, TypeDeclarationKind::Declare);
    assert_eq!(v[1].kind, TypeDeclarationKind::Define);
    assert_eq!(v[1].expression,
        TypeExpression::Union(vec!["int".into(), "bool".into()]));
}

#[test]
fn inline_nominal_identity_is_shared_with_the_alias() {
    accepts("declare type Bar = Z; define type Bar as interface Z {}");
    accepts("declare type Other = T; define type Other as trait T {}");
    accepts("define type Bar as interface Z {} declare type Star = Z;");
    accepts("define type Bar as interface Z {};");
}

#[test]
fn final_semicolon_optional_only_at_end_of_file_for_bare_expressions() {
    accepts("define type One as bool");
    accepts("define type One as bool; declare type Two = One");
    rejects("define type One as bool declare type Two = One", "E_SYNTAX");
    rejects("declare type One = int define type Two as bool", "E_SYNTAX");
}

#[test]
fn separator_confusion_is_rejected() {
    rejects("define type Foo = bool | int;", "E_SYNTAX");
    rejects("declare type Foo as bool;", "E_SYNTAX");
    rejects("define type Foo bool;", "E_SYNTAX");
    rejects("declare type Foo bool;", "E_SYNTAX");
}

#[test]
fn trailing_or_missing_union_arms_are_rejected() {
    rejects("define type Foo as bool |", "E_SYNTAX");
    rejects("define type Foo as | bool", "E_SYNTAX");
    rejects("define type Foo as bool || int", "E_SYNTAX");
}

#[test]
fn duplicate_arms_declarations_and_definitions_are_rejected() {
    rejects("define type Foo as bool | bool;", "E_DUPLICATE");
    rejects("declare type Foo = bool; declare type Foo = bool;", "E_DUPLICATE");
    rejects("define type Foo as int; define type Foo as int;", "E_DUPLICATE");
    rejects("define type A as interface Z {} define type B as interface Z {}", "E_DUPLICATE");
}

#[test]
fn signatures_must_match_definitions() {
    rejects("declare type Foo = bool; define type Foo as int;", "E_SIGNATURE");
    rejects("define type Foo as bool | int; declare type Foo = int | string", "E_SIGNATURE");
}

#[test]
fn aliases_must_not_create_recursion_without_nominal_anchor() {
    rejects("define type A as A", "E_TYPE_CYCLE");
    rejects("declare type A = B; define type B as A", "E_TYPE_CYCLE");
    accepts("declare type Star = ExternalZ;");
}

#[test]
fn builtin_names_are_not_redefined() {
    rejects("define type bool as int", "E_SYNTAX");
    rejects("declare type int = bool", "E_SYNTAX");
    rejects("define type interface as bool", "E_SYNTAX");
}

#[test]
fn unsupported_member_bodies_and_nominal_unions_fail_closed() {
    rejects("define type Bar as interface Z { fnc test(): void; }", "E_UNSUPPORTED");
    rejects("define type Bar as trait T { fnc ping() {} }", "E_UNSUPPORTED");
    rejects("define type Foo as interface Z {} | int", "E_UNSUPPORTED");
    rejects("declare type Foo = interface Z {}", "E_SYNTAX");
}

#[test]
fn detector_ignores_nested_fake_type_declarations() {
    let source = "fnc main(): int { return 4; /* define type Fake as bool */ }";
    assert!(!contains_type_declaration(&lex(source).unwrap()));
    assert!(contains_type_declaration(&lex("define type Real as bool").unwrap()));
}

#[test]
fn parsing_is_not_codegen_success() {
    let source = "define type Foo as bool | int";
    assert!(parse_source(source).is_ok());
    let err = atto_compiler::compile(source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED");
}

#[test]
fn a_negative_type_contract_cannot_emit_wasm() {
    for src in ["define type Foo = bool", "define type Foo as bool |",
                "declare type Foo as bool"] {
        assert!(atto_compiler::compile(src).is_err(), "{src}");
    }
}
