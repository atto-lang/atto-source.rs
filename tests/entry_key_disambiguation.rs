//! Guard collection-key vs computed-key ambiguity while standalone Entry and
//! Symbol guest support remains separately gated. This tests real parser and
//! static-typing behavior; it does NOT claim executable Entry guest support.
use atto_compiler::{
    ast::{ExprKind, Program, Stmt},
    collection_types::{self, LiteralType},
    compile, lexer,
    parser::Parser,
};

fn source(expr: &str) -> String {
    format!("pub fnc main(): int {{ let value = {expr}; return 0; }}")
}

fn parse_expr(expr: &str) -> ExprKind {
    let text = source(expr);
    let program = Parser::new(lexer::lex(&text).unwrap())
        .parse_program()
        .unwrap();
    let Stmt::Bind { init, .. } = &program.funcs[0].body[0] else {
        panic!("expected ordinary binding");
    };
    init.kind.clone()
}

fn program_with_binding(expr: &str) -> Program {
    let text = source(expr);
    Parser::new(lexer::lex(&text).unwrap())
        .parse_program()
        .unwrap()
}

#[test]
fn bare_brackets_are_sequence_literals_not_variable_key_markers() {
    assert!(matches!(parse_expr(r#"["foo"]"#), ExprKind::Array(ref items)
        if items.len() == 1 && matches!(&items[0].kind, ExprKind::Str(s) if s == "foo")));
    assert!(matches!(parse_expr("[true]"), ExprKind::Array(ref items)
        if items.len() == 1 && matches!(&items[0].kind, ExprKind::Bool(true))));
    assert!(matches!(parse_expr("[foo]"), ExprKind::Array(ref items)
        if items.len() == 1 && matches!(&items[0].kind, ExprKind::Var(name) if name == "foo")));
    assert!(matches!(parse_expr("(foo)"), ExprKind::Var(ref name) if name == "foo"));
    assert!(matches!(parse_expr("(foo,)"), ExprKind::Tuple(ref items)
        if items.len() == 1 && matches!(&items[0].kind, ExprKind::Var(name) if name == "foo")));

    for (literal, inner) in [(r#"["foo"]"#, LiteralType::String),
                             ("[true]", LiteralType::Bool)] {
        let typed = collection_types::analyze(&program_with_binding(literal)).unwrap();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].value.ty, LiteralType::List(Box::new(inner)),
            "nonempty unannotated [] chooses List, never an expression-key marker");
    }
}

#[test]
fn record_field_vs_map_expression_key_are_distinct_even_for_identifiers() {
    assert!(matches!(parse_expr("{foo:5}"), ExprKind::Record(ref fields)
        if fields.len() == 1 && fields[0].0 == "foo"));
    assert!(matches!(parse_expr("(foo=5)"), ExprKind::Record(ref fields)
        if fields.len() == 1 && fields[0].0 == "foo"));
    assert!(matches!(parse_expr("{foo=>5}"), ExprKind::Map(ref items)
        if items.len() == 1 && matches!(&items[0].0.kind, ExprKind::Var(name) if name == "foo")));
    assert!(matches!(parse_expr(r#"{"foo":5}"#), ExprKind::Map(ref items)
        if items.len() == 1 && matches!(&items[0].0.kind, ExprKind::Str(s) if s == "foo")));

    let a = collection_types::analyze(&program_with_binding("{foo:5}")).unwrap();
    let b = collection_types::analyze(&program_with_binding("(foo=5)")).unwrap();
    assert_eq!(a[0].value.ty, b[0].value.ty,
        "brace and parenthesized Record syntax must retain identical structure");
    assert!(matches!(&a[0].value.ty, LiteralType::Record(fields)
        if fields.len() == 1 && fields[0].0 == "foo"));

    let err = collection_types::analyze(&program_with_binding("{[true]=>5}"))
        .expect_err("growable List key cannot silently become a hashable Map key");
    assert_eq!(err.code, "E_MAP_KEY");
    let err = collection_types::analyze(&program_with_binding(r#"{["foo"]=>5}"#))
        .expect_err("string List key is not automatically stable/hashable");
    assert_eq!(err.code, "E_MAP_KEY");
}

#[test]
fn unimplemented_entry_and_symbol_syntax_do_not_emit_unsafe_guest_code() {
    // These are normative future features. Current browser compiler MUST
    // reject with no guest artifact until real Entry + Symbol lowering exists.
    for expr in [
        "foo => 5",
        "(foo) => 5",
        r#"["foo"] => 5"#,
        "[true] => 5",
        r#"["foo" => 5, "bar" => 6]"#,
        "bar :: 5",
        "#foo => 5",
    ] {
        let err = compile(&source(expr)).expect_err("no unchecked Entry guest path");
        assert!(
            matches!(err.code, "E_UNSUPPORTED" | "E_SYNTAX" | "E_SEMICOLON" | "E_EXPECT"),
            "unexpected syntax admission for {expr}: {err}"
        );
    }
}
