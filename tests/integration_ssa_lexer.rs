//! Integration regressions for source-owned immutable/explicit-mutable scalar
//! semantics and CR/LF line-comment shielding; no actor or branch lowering.
use oreslang_source_rs::{compile, frontend, mir};

#[test]
fn cr_line_comment_shields_fake_block_starts_before_mutable_guest() {
    for source in [
        "// fake /* CR\rpub fnc main(): int { let x = 40; x += 2; return x; }",
        "// fake /* LF\npub fnc main(): int { mut int x = 41; x = x + 1; return x; }",
        "// fake */ /* CRLF\r\npub fnc main(): int { let x = 43; x -= 1; return x; }",
    ] {
        assert!(!compile(source).unwrap().is_empty(), "{source:?}");
    }
}

#[test]
fn typed_val_remains_immutable_even_after_mutable_ssa_lowering() {
    for body in [
        "val x: int = 7; x = 42; return x;",
        "const x: int = 7; x += 35; return x;",
        "int x = 7; x -= 1; return x;",
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        let error = frontend::parse(&source).unwrap_err();
        assert_eq!(error.class, "immutable_binding", "{body}");
        assert!(compile(&source).is_err(), "{body}");
    }
}

#[test]
fn explicit_mutable_rebinding_cannot_rename_existing_source_bindings() {
    let source = concat!(
        "pub fnc main(): int { const __atto_scalar_rebind_0 = 3; ",
        "let x = 1; x += 38; const prior = x; x = x + 3; ",
        "return prior + __atto_scalar_rebind_0; }"
    );
    let parsed = frontend::parse(source).unwrap();
    let names: Vec<_> = parsed.functions[0].locals.iter().map(|local| local.name.as_str()).collect();
    assert_eq!(names.len(), names.iter().copied().collect::<std::collections::HashSet<_>>().len());
    assert!(mir::lower(parsed).is_ok());
    assert!(!compile(source).unwrap().is_empty());
}

#[test]
fn mutation_without_explicit_write_permission_never_emits_native_guest() {
    for body in [
        "val x: int = 40; x += 2; return x;",
        "const x = 40; x = 42; return x;",
        "let x = 40; const y = x; y = 42; return y;",
        "fnc unknown(): int { return 42; }",
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        assert!(compile(&source).is_err(), "{body}");
    }
}

#[test]
fn deliberately_unsupported_branching_and_actor_surface_fails_closed() {
    for body in [
        "if true then return 42; else return 0; fi",
        "while true { return 42; }",
        "rt borrow x; return 42;",
        "critical { return 42; }",
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        assert!(compile(&source).is_err(), "{body}");
    }
}
