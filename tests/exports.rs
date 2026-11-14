//! File-level export declarations are order-independent and checked by kind.
use atto_compiler::ast::{ExportKind, ExportVisibility};
use atto_compiler::{compile, lexer, parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    parser::Parser::new(lexer::lex(source).expect("lex")).parse_program().expect("parse")
}

const DECLS: &str = r#"
define class Foo as
end
define class Bar as
end
fnc calculate(int n): int { return n + 1; }
pub fnc main(): int { return calculate(41); }
"#;

const EXPORTS: &str = r#"
export pub Foo;
export pub[pkg] class Bar;
export pub fnc calculate;
"#;

#[test]
fn both_export_spellings_and_package_visibility_parse() {
    let program = parse(&format!("{EXPORTS}{DECLS}"));
    assert_eq!(program.exports.len(), 3);
    assert_eq!(program.exports[0].name, "Foo");
    assert_eq!(program.exports[0].kind, None);
    assert_eq!(program.exports[0].visibility, ExportVisibility::Public);
    assert_eq!(program.exports[1].name, "Bar");
    assert_eq!(program.exports[1].kind, Some(ExportKind::Class));
    assert_eq!(program.exports[1].visibility, ExportVisibility::Package);
    assert_eq!(program.exports[2].kind, Some(ExportKind::Fnc));
}

#[test]
fn moving_exports_from_top_to_bottom_preserves_binary_and_metadata() {
    let top = compile(&format!("{EXPORTS}{DECLS}")).expect("top exports");
    let bottom = compile(&format!("{DECLS}{EXPORTS}")).expect("bottom exports");
    assert_eq!(top, bottom, "export resolution must be source-order independent");
    assert!(top.windows(b"atto.exports.v1".len()).any(|w| w == b"atto.exports.v1"));
    assert!(top.windows(b"calculate".len()).any(|w| w == b"calculate"));
}

#[test]
fn explicit_and_inferred_export_kinds_agree() {
    let src = format!(
        "export pub class Foo;\nexport pub Bar;\nexport pub[pkg] fnc calculate;\n{DECLS}"
    );
    assert!(compile(&src).is_ok());
}

#[test]
fn actors_and_enums_are_valid_export_targets_in_the_ast() {
    let p = parse(
        "export pub actor Worker;\nexport pub enum Status;\n         define actor Worker as end\n         define enum Status as Ready end\n         pub fnc main(): int { return 0; }"
    );
    assert_eq!(p.exports[0].kind, Some(ExportKind::Actor));
    assert_eq!(p.exports[1].kind, Some(ExportKind::Enum));
}

#[test]
fn bad_targets_and_duplicate_exports_fail_closed() {
    let cases = [
        ("export pub Missing;\npub fnc main(): int { return 0; }", "E_UNBOUND"),
        ("export pub fnc Foo;\n", "E_UNBOUND"),
        ("export pub fnc Foo;\ndefine class Foo as end\npub fnc main(): int { return 0; }", "E_EXPORT_KIND"),
        ("export pub Foo;\nexport pub class Foo;\ndefine class Foo as end\npub fnc main(): int { return 0; }", "E_DUPLICATE"),
        ("export pub[pkg] Ghost;\npub fnc main(): int { return 0; }", "E_UNBOUND"),
    ];
    for (src, want) in cases {
        let err = compile(src).expect_err(src);
        assert_eq!(err.code, want, "{src}: {err}");
    }
}

#[test]
fn invalid_qualifiers_and_missing_separators_do_not_consume_declarations() {
    for src in [
        "export Foo;\npub fnc main(): int { return 0; }",
        "export pub[world] Foo;\npub fnc main(): int { return 0; }",
        "export pub Foo\ndefine class Foo as end\npub fnc main(): int { return 0; }",
        "pub fnc main(): int { export pub Foo; return 0; }",
    ] {
        assert!(compile(src).is_err(), "should reject: {src}");
    }
}

#[test]
fn export_resolves_all_overloads_by_name() {
    let source = "export pub fnc value;\n        fnc value(): int { return 1; }\n        fnc value(int x): int { return x; }\n        pub fnc main(): int { return value() + value(2); }";
    assert!(compile(source).is_ok());
}
