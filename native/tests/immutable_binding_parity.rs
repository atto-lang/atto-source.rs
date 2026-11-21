//! Scalar immutable-binding parity admission. These tests exercise the real
//! Rust parser, MIR validation, effect exporter and Cranelift object emitter.
//! Cross-backend Java/LLVM native execution remains a separate evidence gate.
use oreslang_source_rs::{compile, compile_with_effect_ir, frontend, mir};

fn source(binding: &str) -> String {
    format!("pub fnc main(): int {{ {binding} return x + 2; }}")
}

#[test]
fn canonical_immutable_spellings_have_identical_ast_and_effects() {
    let forms = [
        "const x: int = 40;",
        "const int x = 40;",
        "const x = 40;",
        "int x = 40;",
        "val x = 40;",
        "val x: int = 40;",
    ];
    let baseline = frontend::parse(&source(forms[0])).unwrap();
    let (_, expected_effects) = compile_with_effect_ir(&source(forms[0])).unwrap();
    for form in forms {
        let input = source(form);
        assert_eq!(frontend::parse(&input).unwrap(), baseline, "{form}");
        let (object, effects) = compile_with_effect_ir(&input).unwrap();
        assert!(!object.is_empty(), "{form}");
        assert_eq!(effects, expected_effects, "{form}");
    }
}

#[test]
fn scalar_immutable_binding_fixture_emits_object() {
    let src = include_str!("fixtures/immutable-bindings.atto");
    let object = compile(src).expect("canonical scalar fixture must compile");
    assert!(!object.is_empty());
    let (paired_object, effects) = compile_with_effect_ir(src).unwrap();
    assert!(!paired_object.is_empty());
    assert_eq!(effects["schema_version"], 1);
}

#[test]
fn forward_scalar_calls_in_immutable_initializers_are_checked() {
    let src = "pub fnc main(): int { int x = plus_one(41); return x; } fnc plus_one(int n): int { return n + 1; }";
    assert!(!compile(src).unwrap().is_empty());
}

#[test]
fn duplicate_bindings_across_spellings_are_denied() {
    for declarations in [
        "int x = 1; const x = 2;",
        "val x = 1; const int x = 2;",
        "const int x = 1; val x = 2;",
        "const x: int = 1; int x = 2;",
    ] {
        let src = format!("pub fnc main(): int {{ {declarations} return x; }}");
        let ast = frontend::parse(&src).unwrap();
        assert_eq!(mir::lower(ast).unwrap_err().class, "duplicate_declaration");
        assert!(compile(&src).unwrap_err().starts_with("duplicate_declaration"));
    }
}

#[test]
fn immutable_initializers_cannot_read_undefined_or_future_locals() {
    for bindings in [
        "int x = missing; return x;",
        "val x = y; const y = 42; return x;",
        "const x = x + 1; return x;",
    ] {
        let src = format!("pub fnc main(): int {{ {bindings} }}");
        assert!(compile(&src).unwrap_err().starts_with("unknown_name"));
    }
}

#[test]
fn invalid_or_mutable_declarations_fail_closed() {
    for bindings in [
        "int x; return 1;",
        "const x; return 1;",
        "val x = ; return 1;",
        "let x = 42; return x;",
        "mut int x = 42; return x;",
        "const val x = 42; return x;",
        "const x: bool = true; return 1;",
    ] {
        assert!(compile(&format!("pub fnc main(): int {{ {bindings} }}")).is_err(), "{bindings}");
    }
}

#[test]
fn malformed_literals_in_canonical_locals_are_denied() {
    for declaration in [
        "int x = 1__0;",
        "val x = 1_;",
        "const int x = 9223372036854775808;",
    ] {
        let src = format!("pub fnc main(): int {{ {declaration} return x; }}");
        assert!(compile(&src).is_err(), "{declaration}");
    }
}

#[test]
fn slash_star_inside_line_comment_does_not_open_block_comment() {
    for header in [
        "// /* block starts only inside the line comment\\n",
        "// /* unclosed pseudo-comment\\n// more text\\n",
        "// */ and /* are just text\\n",
    ] {
        let prefix = header.replace("\\n", "\n");
        let src = format!("{prefix}pub fnc main(): int {{ return 42; }}");
        assert!(!compile(&src).unwrap().is_empty(), "{header}");
    }
}

#[test]
fn genuine_nested_block_comments_still_work() {
    let source = "/* // not a line comment /* nested */ outer */ pub fnc main(): int { return 42; }";
    assert!(!compile(source).unwrap().is_empty());
}

#[test]
fn unclosed_real_block_comment_remains_an_error() {
    let src = "// /* ignored\n/* real unclosed\npub fnc main(): int { return 42; }";
    let err = frontend::parse(src).unwrap_err();
    assert_eq!(err.class, "unterminated_comment");
}
