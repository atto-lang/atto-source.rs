//! Syntax-only collection literal contract. These cases assert AST recognition,
//! canonical type spelling, diagnostics and a fail-closed browser guest gate.
//! Passing them is NOT evidence that collection values execute in guest Wasm.
use atto_compiler::{ast::{ExprKind, Program, Stmt, Type}, compile, diag::Diagnostic, lexer, parser::Parser};

fn parse_binding(text: &str) -> Result<Program, Diagnostic> {
    let source = format!("fnc main(): int {{\n{text}\nreturn 0;\n}}");
    Parser::new(lexer::lex(&source)?).parse_program()
}

fn init_kind(program: &Program) -> &ExprKind {
    match &program.funcs[0].body[0] {
        Stmt::Bind { init, .. } => &init.kind,
        _ => panic!("first statement should be a binding"),
    }
}

#[test]
fn all_requested_forms_parse_without_guest_execution() {
    let cases = [
        ("let Record foo = {bar: []};", "Record"),
        ("let Map foo = {\n \"1\" => 1\n \"2\" => 3\n};", "Map"),
        ("let List foo = [];", "List"),
        ("let Array<Record> foo = [{job: 1}, {car: \"str\"}, {x: true}];", "Array"),
        ("let Tuple foo = ();", "Tuple"),
        ("let tuple foo = ();", "Tuple"),
        ("let array<record> foo = [{job: 1}, {car: \"str\"}, {x: true}];", "Array"),
        ("let record foo = {bar: []};", "Record"),
        ("let map foo = {\"a\" => 1};", "Map"),
        ("let list foo = [];", "List"),
        ("let foo: Array<Record> = [{job: 1}];", "Array"),
    ];
    for (src, canonical) in cases {
        let program = parse_binding(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        match &program.funcs[0].body[0] {
            Stmt::Bind { ty: Some(Type::Named(i)), .. } => {
                assert!(program.type_names[*i as usize].0.starts_with(
                    &format!("@builtin/{canonical}")), "{src}");
            }
            _ => panic!("{src}: expected a collection-typed binding"),
        }
    }
}

#[test]
fn brace_entries_distinguish_maps_from_records() {
    let records = parse_binding("let Record foo = {bar: [], \"quoted\": {nested: true}};").unwrap();
    match init_kind(&records) {
        ExprKind::Record(entries) => {
            assert_eq!(entries.len(), 2);
            assert!(matches!(&entries[0].1.kind, ExprKind::Array(_)));
            assert!(matches!(&entries[1].1.kind, ExprKind::Record(_)));
        }
        _ => panic!("record entry AST was not preserved"),
    }
    let maps = parse_binding("let Map foo = {\n\"1\" => 1\n\"2\" => 3\n};").unwrap();
    assert!(matches!(init_kind(&maps), ExprKind::Map(m) if m.len() == 2));
    let empty = parse_binding("let Map foo = {};").unwrap();
    assert!(matches!(init_kind(&empty), ExprKind::EmptyBraces));
}

#[test]
fn tuple_arity_is_not_grouping() {
    assert!(matches!(init_kind(&parse_binding("let Tuple t = ();").unwrap()),
        ExprKind::Tuple(t) if t.is_empty()));
    assert!(matches!(init_kind(&parse_binding("let Tuple t = (1,);").unwrap()),
        ExprKind::Tuple(t) if t.len() == 1));
    assert!(matches!(init_kind(&parse_binding("let Tuple t = (1, true);").unwrap()),
        ExprKind::Tuple(t) if t.len() == 2));
    assert!(!matches!(init_kind(&parse_binding("let int t = (1);").unwrap()),
        ExprKind::Tuple(_)));
}

#[test]
fn reject_bad_fields_keys_and_missing_separators() {
    for (source, expected) in [
        ("let Record x = {a: 1, a: 2};", "E_DUPLICATE"),
        ("let Record x = {a: 1, \"a\": 2};", "E_DUPLICATE"),
        ("let Map x = {\"a\" => 1, \"a\" => 2};", "E_DUPLICATE"),
        ("let Map x = {a: 1, \"b\" => 2};", "E_LITERAL_KIND"),
        ("let Record x = {a: 1, \"b\" => 2};", "E_LITERAL_KIND"),
        ("let Record x = {a: 1 b: 2};", "E_SYNTAX"),
        ("let Map x = {\"a\" => 1 \"b\" => 2};", "E_SYNTAX"),
        ("let Map x = {\"a\" => 1;", "E_SYNTAX"),
    ] {
        let error = parse_binding(source).expect_err(source);
        assert_eq!(error.code, expected, "{source}: {error}");
    }
}

#[test]
fn reject_bad_generic_arities() {
    for source in [
        "let Array<int,string> a = [];",
        "let array<int,string> a = [];",
        "let Map<string> m = {};",
        "let Record<int> r = {};",
        "let List<> l = [];",
    ] {
        assert_eq!(parse_binding(source).expect_err(source).code, "E_TYPE_ARITY", "{source}");
    }
}

#[test]
fn never_emit_unchecked_guest_collection_values() {
    for source in [
        "let Record foo = {bar: []};",
        "let Map foo = {\"1\" => 1};",
        "let List foo = [];",
        "let Array<Record> foo = [{job: 1}];",
        "let tuple foo = ();",
    ] {
        let full_source = format!("fnc main(): int {{\n{source}\nreturn 0;\n}}");
        let diagnostic = compile(&full_source).expect_err("no guest collection allocation exists");
        assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{source}: {diagnostic}");
    }
}
