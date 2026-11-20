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

#[test]
fn nominal_entry_fat_arrow_parses_without_changing_map_items() {
    // Entry literal: deliberately synthetic nominal factory AST, never Tuple
    // or Record. This is a parse-only gate, not executable guest support.
    let p = parse_binding("let Entry<string, int> entry = \"foo\" => 5;")
        .expect("typed nominal Entry literal syntax");
    match init_kind(&p) {
        ExprKind::Call(name, args) => {
            assert_eq!(name, "@builtin/Entry.of");
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0].kind, ExprKind::Str(s) if s == "foo"));
            assert!(matches!(&args[1].kind, ExprKind::Int(Some(5))));
        }
        _ => panic!("standalone => must represent an Entry, not Map/Tuple/Record"),
    }
    let p = parse_binding(
        "let Map<string,int> x = {\"foo\" => 5, \"bar\" => 7};"
    ).expect("map braces still own their => delimiters");
    assert!(matches!(init_kind(&p), ExprKind::Map(m) if m.len() == 2));
    let p = parse_binding(
        "let Map<string,Entry<string,int>> x = {\"foo\" => (\"bar\" => 7)};"
    ).expect("a grouped nominal Entry is a Map value");
    assert!(matches!(init_kind(&p), ExprKind::Map(m) if matches!(&m[0].1.kind,
        ExprKind::Call(name, args) if name == "@builtin/Entry.of" && args.len() == 2)));
}

#[test]
fn nominal_entry_syntax_rejects_ambiguous_arrow_chains_and_generic_arity() {
    let bad = parse_binding("let Entry<string,int> e = \"a\" => 1 => 2;")
        .expect_err("unparenthesized chained arrows are ambiguous");
    assert_eq!(bad.code, "E_SYNTAX");
    assert_eq!(
        parse_binding("let Entry<int> e = 1 => 2;")
            .expect_err("Entry requires exactly K and V").code,
        "E_TYPE_ARITY"
    );
    // The module/tenant-level let binding and actual mutable Map guest
    // operations require their own verified compiler/runtime implementation.
    // This parser test intentionally does not pretend to execute either.
}

#[test]
fn entry_literal_alone_cannot_bypass_guest_type_and_ownership_checks() {
    let src = "fnc main(): int {
        let Entry<string,int> e = \"foo\" => 5;
        return 0;
    }";
    let diag = compile(src).expect_err("nominal Entry guest lowering not yet implemented");
    assert_eq!(diag.code, "E_UNSUPPORTED");
}

#[test]
fn both_whole_entry_ascriptions_have_identical_nominal_ast() {
    let first = parse_binding(
        r#"let entry1 = ("foo" => 5) as Entry<string, int>;"#
    ).expect("grouped Entry ascription");
    let second = parse_binding(
        r#"let entry1 = "foo" => 5 (as Entry<string, int>);"#
    ).expect("trailing Entry ascription");
    fn signature(p: &Program) -> (String, String, usize) {
        match init_kind(p) {
            ExprKind::Call(name, args) if args.len() == 1 => {
                match &args[0].kind {
                    ExprKind::Call(inner, parts) if inner == "@builtin/Entry.of" => {
                        assert!(matches!(&parts[0].kind, ExprKind::Str(s) if s == "foo"));
                        assert!(matches!(&parts[1].kind, ExprKind::Int(Some(5))));
                        (name.clone(), inner.clone(), parts.len())
                    },
                    _ => panic!("cast must wrap an immutable Entry factory"),
                }
            }
            _ => panic!("whole-entry ascription must use canonical typed AST"),
        }
    }
    let one = signature(&first);
    let two = signature(&second);
    assert_eq!(one, two);
    assert_eq!(one.0, "@builtin/Entry.ascribe<@builtin/Entry<string,int>>");
}

#[test]
fn entry_type_ascriptions_reject_non_entry_casts_and_malformed_syntax() {
    for src in [
        r#"let e = ("foo" => 5) as Map<string,int>;"#,
        r#"let e = "foo" => 5 (as Map<string,int>);"#,
        r#"let e = 5 as Entry<string,int>;"#,
    ] {
        let e = parse_binding(src).expect_err(src);
        assert_eq!(e.code, "E_TYPE", "{src}: {e}");
    }
    for src in [
        r#"let e = ("foo" => 5) as Entry<string>;"#,
        r#"let e = "foo" => 5 (as Entry<string>);"#,
    ] {
        assert_eq!(parse_binding(src).unwrap_err().code, "E_TYPE_ARITY", "{src}");
    }
    for src in [
        r#"let e = "foo" => 5 (as Entry<string,int>;"#,
        r#"let e = "foo" => 5 (as Entry<string,int>,);"#,
    ] {
        assert!(parse_binding(src).is_err(), "{src}");
    }
}

#[test]
fn bracket_lvalues_preserve_exact_index_target_without_guessing_record_or_map() {
    let cases = [
        r#"let mut Map<string,int> x = {"foo" => 5}; x["foo"] = 3;"#,
        r#"let mut Record x = {foo: 5}; x["foo"] = 3;"#,
        r#"let x = new Point(); x["foo"] = 3;"#,
    ];
    for case in cases {
        let p = parse_binding(case).unwrap_or_else(|e| panic!("{case}: {e}"));
        match &p.funcs[0].body[1] {
            Stmt::Expr(e) => match &e.kind {
                ExprKind::Call(name, args) if name == "@builtin/checked-index-write" => {
                    assert_eq!(args.len(), 2);
                    assert!(matches!(&args[1].kind, ExprKind::Int(Some(3))));
                    match &args[0].kind {
                        ExprKind::Index(receiver, key) => {
                            assert!(matches!(&receiver.kind, ExprKind::Var(n) if n == "x"));
                            assert!(matches!(&key.kind, ExprKind::Str(s) if s == "foo"));
                        }
                        _ => panic!("must keep unlowered Index lvalue to resolve after typing"),
                    }
                }
                _ => panic!("synthetic index write must be a checked, unspellable intrinsic"),
            }
            _ => panic!("expected a single checked index assignment statement"),
        }
    }
}

#[test]
fn unchecked_index_assignment_never_emits_guest_code() {
    let invalid = r#"fnc main(): int {
        let int x = 1;
        x["foo"] = 3;
        return x;
    }"#;
    let err = compile(invalid).expect_err("no unproven index writer should execute");
    assert_eq!(err.code, "E_UNSUPPORTED");
}
