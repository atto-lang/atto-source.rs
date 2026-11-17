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
fn builtin_aliases_share_interned_identity_within_one_program() {
    let program = parse_binding(
        "let Array<Record> first = [];
         let array<record> second = [];
         let Tuple t = ();
         let tuple u = ();"
    ).unwrap();
    let mut refs = Vec::new();
    for stmt in &program.funcs[0].body[..4] {
        if let Stmt::Bind { ty: Some(Type::Named(id)), .. } = stmt {
            refs.push(*id);
        } else {
            panic!("expected four explicitly typed collection bindings");
        }
    }
    assert_eq!(refs[0], refs[1], "Array aliases must share type identity");
    assert_eq!(refs[2], refs[3], "Tuple aliases must share type identity");
    assert_ne!(refs[0], refs[2], "distinct collection types must remain distinct");
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
        ("let Map x = {-1 => 1, -1 => 2};", "E_DUPLICATE"),
        ("let Map x = {-0 => 1, 0 => 2};", "E_DUPLICATE"),
        ("let Map x = {+1 => 1, 1 => 2};", "E_DUPLICATE"),
        ("let Map x = {-9223372036854775808 => 1, -9223372036854775808 => 2};", "E_DUPLICATE"),
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
fn lowercase_aliases_have_identical_canonical_type_identity() {
    fn name_of(source: &str) -> String {
        let p = parse_binding(source).unwrap();
        match &p.funcs[0].body[0] {
            Stmt::Bind { ty: Some(Type::Named(i)), .. } =>
                p.type_names[*i as usize].0.clone(),
            _ => panic!("expected generic collection type"),
        }
    }
    assert_eq!(name_of("let Array<Record> a = [];"),
               name_of("let array<record> a = [];"));
    assert_eq!(name_of("let Map<string,int> m = {};"),
               name_of("let map<string,int> m = {};"));
    assert_eq!(name_of("let Tuple t = ();"),
               name_of("let tuple t = ();"));
}

#[test]
fn generic_nesting_is_bounded_before_stack_exhaustion() {
    let name = format!("{}int{}", "Array<".repeat(140), ">".repeat(140));
    let source = format!("let {name} xs = [];");
    assert_eq!(parse_binding(&source).expect_err("excessive type depth").code,
               "E_NESTING");
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


#[test]
fn bracket_access_keeps_receiver_and_literal_key_distinct_from_dot_members() {
    let program = parse_binding(
        "let Record headers = {\"content-type\": \"application/json\", status_code: 200};\n\
         let string content = headers[\"content-type\"];\n\
         let int status = headers.status_code;"
    ).expect("bracket record access should be admitted as syntax");
    match &program.funcs[0].body[1] {
        Stmt::Bind { init, .. } => match &init.kind {
            ExprKind::Index(base, key) => {
                assert!(matches!(&base.kind, ExprKind::Var(name) if name == "headers"));
                assert!(matches!(&key.kind, ExprKind::Str(name) if name == "content-type"));
            }
            _ => panic!("bracket access must not be converted to Member"),
        },
        _ => panic!("expected bracket binding"),
    }
    match &program.funcs[0].body[2] {
        Stmt::Bind { init, .. } =>
            assert!(matches!(&init.kind, ExprKind::Member(_, field) if field == "status_code")),
        _ => panic!("expected dot-member binding"),
    }
    let map = parse_binding(
        "let Map<string,int> m = {\"\" => 0, \"retry-count\" => 3};\n\
         let int x = m[\"retry-count\"];"
    ).unwrap();
    assert!(matches!(&map.funcs[0].body[1],
        Stmt::Bind { init, .. } if matches!(&init.kind, ExprKind::Index(_, _))));
}

#[test]
fn computed_bracket_keys_are_not_misparsed_as_static_record_fields() {
    let program = parse_binding(
        "let Record headers = {\"content-type\": \"ok\"};\n\
         let string key = \"content-type\";\n\
         let string found = headers[key];"
    ).unwrap();
    assert!(matches!(&program.funcs[0].body[2],
        Stmt::Bind { init, .. } if matches!(&init.kind, ExprKind::Index(_, key)
            if matches!(&key.kind, ExprKind::Var(name) if name == "key"))));
    let bad = parse_binding("let Record x = {a: 1}; let int y = x[\"a\";");
    assert_eq!(bad.unwrap_err().code, "E_SYNTAX", "unclosed brackets reject");
}

#[test]
fn quoted_record_key_duplicates_reject_after_string_decoding() {
    for source in [
        "let Record x = {\"\": 1, \"\": 2};",
        "let Record x = {a: 1, \"a\": 2};",
    ] {
        assert_eq!(parse_binding(source).unwrap_err().code, "E_DUPLICATE", "{source}");
    }
    let program = parse_binding("let Record x = {\"\": 1, \"content-type\": 2};").unwrap();
    assert!(matches!(init_kind(&program), ExprKind::Record(entries)
        if entries[0].0 == "" && entries[1].0 == "content-type"));
}

#[test]
fn indexing_is_never_emitted_as_unverified_guest_wasm() {
    let source = "fnc main(): int { let int value = 3; return value[\"key\"]; }";
    let diagnostic = compile(source).expect_err("no checked runtime indexing");
    assert_eq!(diagnostic.code, "E_UNSUPPORTED");
}
