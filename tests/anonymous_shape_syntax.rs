//! Anonymous structural shapes: production AST retention and checked fail-closed ABI.
//! These tests do NOT claim compiled/guest-executed shape formals or maps.
use atto_compiler::{compile, lexer::lex, parser::Parser, structural_syntax as syn};
use atto_compiler::ast::Type;

#[test]
fn bare_shapes_and_explicit_structural_shapes_have_identical_type_identity() {
    for (bare, explicit, long) in [
        ("({foo: string} p)", "(~{foo: string} p)", "(structural {foo: string} p)"),
        ("(p: {foo: string | int})", "(p: ~ {foo: string | int})",
         "(p: structural {foo: string | int})"),
        ("(p: {foo: \"dog\"})", "(p: ~ {foo: \"dog\"})",
         "(p: structural {foo: \"dog\"})"),
        ("({foo: 42, status: bool} p)", "(~{foo: 42, status: bool} p)",
         "(structural {foo: 42, status: bool} p)"),
        ("({foo: {nested: bool}} p)", "(~{foo: {nested: bool}} p)",
         "(structural {foo: {nested: bool}} p)"),
    ] {
        let a = syn::parse_formals(bare).unwrap();
        assert_eq!(a, syn::parse_formals(explicit).unwrap(), "{bare}");
        assert_eq!(a, syn::parse_formals(long).unwrap(), "{bare}");
        assert!(a[0].ty.structural, "{bare}");
        assert!(a[0].ty.canonical.starts_with("~{"), "{bare}");
    }
}

#[test]
fn literal_and_union_types_do_not_collapse_to_broad_field_types() {
    let ty = |s: &str| syn::parse_formals(s).unwrap()[0].ty.canonical.clone();
    assert_ne!(ty("(p: {foo: \"dog\"})"), ty("(p: {foo: string})"));
    assert_ne!(ty("(p: {foo: string | int})"), ty("(p: {foo: string})"));
    assert_ne!(ty("(p: {foo: 42})"), ty("(p: {foo: int})"));
    assert_eq!(ty("(p: {foo: \"dog\" | \"cat\"})"),
               "~{foo:\"cat\"|\"dog\"}");
}

#[test]
fn bare_shapes_are_value_patterns_without_is_and_type_patterns_with_is() {
    let value = syn::parse_pattern("on {foo: \"dog\", status: _} as val p").unwrap();
    let explicit = syn::parse_pattern("on ~{foo: \"dog\", status: _} as val p").unwrap();
    assert_eq!(value, explicit);
    assert!(matches!(value.kind, syn::PatternKind::ValueFields(_)));
    let shape = syn::parse_pattern("on is {foo: string | int} as val p").unwrap();
    assert_eq!(shape, syn::parse_pattern("on is ~ {foo: string | int} as val p").unwrap());
    assert!(matches!(shape.kind, syn::PatternKind::TypeTest(_)));
    let singleton = syn::parse_pattern("on is {foo: \"dog\"} as val p").unwrap();
    assert!(matches!(singleton.kind, syn::PatternKind::TypeTest(_)));
    assert!(syn::parse_pattern("on {foo: string}").is_err());
    assert!(syn::parse_pattern("on is {foo: _}").is_err());
}

#[test]
fn optional_presence_stays_distinct_from_type_shapes() {
    let value = syn::parse_pattern("on {foo?: \"dog\", status: _}").unwrap();
    let syn::PatternKind::ValueFields(fields) = value.kind else { panic!("value pattern"); };
    assert_eq!(fields.len(),2);
    assert!(fields[0].optional);
    assert!(!fields[1].optional);
    for bad in ["(p: {foo?: string})","(p: {foo: _})","(p: {})",
                "(p: {foo: int, foo: string})","(p: {foo: string,})",
                "(p: {foo: string |})"] {
        assert!(syn::parse_formals(bad).is_err(), "{bad}");
    }
    for bad in ["on {foo?:}","on is {foo?: string}",
                "on {foo: string}","on {foo: effectful()}"] {
        assert!(syn::parse_pattern(bad).is_err(), "{bad}");
    }
}

#[test]
fn structural_row_fields_and_union_arms_canonicalize_as_unordered_sets() {
    let ty = |src: &str| syn::parse_formals(src).unwrap()[0].ty.canonical.clone();
    assert_eq!(ty("(p: {a: int, b: string})"),
               ty("(p: {b: string, a: int})"));
    assert_eq!(ty("(p: {foo: string | int})"),
               ty("(p: {foo: int | string})"));
    assert_eq!(ty("(p: {foo: int | int | string})"),
               ty("(p: {foo: string | int})"));
    assert_eq!(ty("(p: {foo: {z: string, a: bool} | string})"),
               ty("(p: {foo: string | {a: bool, z: string}})"));
    assert_ne!(ty("(p: {foo: int})"), ty("(p: {bar: int})"));
    assert_ne!(ty("(p: {foo: int})"), ty("(p: {foo: string})"));
    assert_eq!(ty("(p: {foo: -0})"), ty("(p: {foo: 0})"));
}

#[test]
fn singleton_types_cannot_leak_into_unrelated_type_positions() {
    for invalid in [
        "(p: \"dog\")", "(p: 42)", "(p: -1)", "(p: true)",
        "(p: ~int)", "(p: structural bool)", "(p: ~\"dog\")",
        "(p: {foo: ~\"dog\"})", "(p: {foo: string | })",
        "(p: {foo: \"dog\" | })", "(p: {foo: 9223372036854775808})",
        "(p: {foo: -9223372036854775809})",
    ] {
        assert!(syn::parse_formals(invalid).is_err(), "{invalid}");
    }
    let got = syn::parse_formals("(p: {foo: -9223372036854775808})").unwrap();
    assert!(got[0].ty.canonical.contains("-9223372036854775808"));
    assert_eq!(
        syn::parse_formals("(p: {foo: \"dog\" | string})").unwrap()[0].ty.canonical,
        syn::parse_formals("(p: {foo: string | \"dog\"})").unwrap()[0].ty.canonical
    );
}

#[test]
fn syntax_only_shape_parser_rejects_deeply_nested_or_oversized_types() {
    let deep = format!("(p: {}int{})", "{x: ".repeat(48), "}".repeat(48));
    assert_eq!(syn::parse_formals(&deep).unwrap_err().code, "E_NESTING");
    let many = (0..34).map(|i| format!("f{i}: int"))
        .collect::<Vec<_>>().join(", ");
    assert_eq!(
        syn::parse_formals(&format!("(p: {{{many}}})")).unwrap_err().code,
        "E_SOURCE_LIMIT"
    );
    let huge_union = (0..34).map(|i| format!("T{i}"))
        .collect::<Vec<_>>().join(" | ");
    assert_eq!(
        syn::parse_formals(&format!("(p: {{foo: {huge_union}}})"))
            .unwrap_err().code,
        "E_SOURCE_LIMIT"
    );
}

#[test]
fn production_compiler_rejects_unlowered_shape_call_abi_before_wasm() {
    for formal in [
        "{foo: string} p",
        "p: {foo: \"dog\"}",
        "p: {foo: string | int}",
        "p: ~ {foo: string}",
        "p: structural {foo: string}",
    ] {
        let source = format!(
            "pub fnc inspect({formal}): int {{ return 42; }} pub fnc main(): int {{ return 42; }}"
        );
        let error = compile(&source).expect_err("shape ABI not available yet");
        assert_eq!(error.code, "E_UNSUPPORTED", "{formal}: {error}");
        assert!(error.message.contains("projected-class ABI"), "{formal}: {error}");
    }
}


#[test]
fn production_parser_preserves_anonymous_formals_without_nominal_erasure() {
    for spelling in [
        "{foo: string} p",
        "p: {foo: \"dog\"}",
        "p: {foo: string | int}",
        "p: ~ {foo: string}",
        "p: structural {foo: string}",
        "p: {foo: {bar: bool}}",
    ] {
        let src = format!(
            "pub fnc inspect({spelling}): int {{ return 42; }} pub fnc main(): int {{ return 42; }}"
        );
        let program = Parser::new(lex(&src).unwrap()).parse_program().unwrap();
        let p = &program.funcs[0].params[0];
        assert!(p.structural, "{spelling}");
        let Type::Named(i) = p.ty else { panic!("shape must retain a reserved, unresolved type identity"); };
        let canonical = &program.type_names[i as usize].0;
        assert!(canonical.starts_with("~{{"), "{spelling}: {canonical}");
        assert_eq!(canonical,
            &syn::parse_formals(&format!("({spelling})")).unwrap()[0].ty.canonical);
        let err = compile(&src).expect_err("no projected shape ABI may emit a guest");
        assert_eq!(err.code, "E_UNSUPPORTED", "{spelling}: {err}");
    }
}

#[test]
fn production_shape_identity_is_order_independent_and_literal_preserving() {
    let src = "pub fnc a(p: {b: int | string, a: \"dog\"}): int { return 1; }
               pub fnc b(p: ~{a: \"dog\", b: string | int}): int { return 2; }
               pub fnc c(p: {a: string, b: int | string}): int { return 3; }
               pub fnc main(): int { return 42; }";
    let program = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    let ids = program.funcs.iter().take(3).map(|f| match f.params[0].ty {
        Type::Named(i) => i,
        _ => panic!("shape formal must retain a canonical identity"),
    }).collect::<Vec<_>>();
    assert_eq!(ids[0], ids[1], "field/union order and tilde must not change identity");
    assert_ne!(ids[0], ids[2], "singleton string may not widen to string implicitly");
    let err = compile(src).expect_err("unlowered ABI must fail closed");
    assert_eq!(err.code, "E_UNSUPPORTED");
}

#[test]
fn production_shape_syntax_validates_fields_before_abi_gate() {
    for (formal, code) in [
        ("p: {foo: int, foo: string}", "E_DUPLICATE"),
        ("p: {foo?: string}", "E_UNSUPPORTED_TYPE"),
        ("p: {foo: _}", "E_TYPE"),
        ("p: {}", "E_SYNTAX"),
        ("p: {foo: string |}", "E_SYNTAX"),
        ("p: {foo: 9223372036854775808}", "E_INTEGER_RANGE"),
    ] {
        let src = format!("pub fnc test({formal}): int {{ return 42; }}");
        let err = compile(&src).expect_err("malformed shape must not reach runtime ABI gate");
        assert_eq!(err.code, code, "{formal}: {err}");
    }
}

#[test]
fn production_routine_and_method_accept_syntax_but_not_unproved_abi() {
    for src in [
        "routine inspect(p: {foo: string}): int { return 42; } pub fnc main(): int { return 42; }",
        "define class Service as pub evaluate(p: {foo: string | int}): int { return 42; } end pub fnc main(): int { return 42; }",
        "define class Service as pub static fnc evaluate({foo: \"dog\"} p): int { return 42; } end pub fnc main(): int { return 42; }",
    ] {
        let err = compile(src).expect_err("class/record/map projected view ABI not yet proven");
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}: {err}");
        assert!(err.message.contains("projected-class ABI"));
    }
}
