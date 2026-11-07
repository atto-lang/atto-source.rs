//! Exact shared source plus Rust/Cranelift immutable-binding parity.
//! Java, LLVM and Wasm carry the same canonical add.atto Git blob.
//! Compilation is not, by itself, native guest execution evidence.
use oreslang_source_rs::{compile, frontend, mir};

const SHARED_ATTO: &str = include_str!("../contracts/atto-parity-v1/add.atto");

fn checked(source: &str) -> Result<(), String> {
    let parsed = frontend::parse(source).map_err(|e| e.to_string())?;
    mir::lower(parsed).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn canonical_atto_blob_is_byte_identical_to_java_llvm_wasm() {
    assert_eq!(SHARED_ATTO, "pub fnc main(): int { return 40 + 2; }\n");
    assert!(compile(SHARED_ATTO).is_ok());
}

#[test]
fn immutable_default_forms_all_compile_from_validated_scalar_mir() {
    let cases = [
        ("int-default", "int x = 7; return x;"),
        ("const-inferred", "const x = 7; return x;"),
        ("const-prefix-type", "const int x = 7; return x;"),
        ("const-colon-type", "const x: int = 7; return x;"),
        ("val-inferred", "val x = 7; return x;"),
        ("dependent-default", "int x = 7; int y = x * 6; return y;"),
        ("dependent-const", "int x = 7; const y = x * 6; return y;"),
        ("dependent-val", "const x = 7; val y = x * 6; return y;"),
        ("call-in-initializer", "const x = add(20,22); return x;"),
    ];
    for (name, body) in cases {
        let source = format!(
            "pub fnc main(): int {{ {body} }} fnc add(int a,int b): int {{ return a+b; }}"
        );
        assert!(checked(&source).is_ok(), "{name}: should typecheck");
        assert!(compile(&source).is_ok(), "{name}: should emit native object");
    }
}

#[test]
fn immutable_reassignments_have_a_specific_diagnostic() {
    for body in [
        "int x = 7; x = 8; return x;",
        "const x = 7; x = 8; return x;",
        "const int x = 7; x += 1; return x;",
        "val x = 7; x -= 1; return x;",
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        let error = checked(&source).unwrap_err();
        assert!(error.starts_with("immutable_binding"), "{body}: {error}");
    }
}

#[test]
fn binding_initializers_remain_closed_world_and_ordered() {
    for (body, diagnostic) in [
        ("int x = x + 1; return x;", "unknown_name"),
        ("int x = y; int y = 7; return x;", "unknown_name"),
        ("int x = 7; const x = 8; return x;", "duplicate_declaration"),
        ("const x = missing; return x;", "unknown_name"),
        ("const x; return 7;", "syntax_error"),
        ("int x = 7; rt mut borrow x; return x;", "syntax_error"),
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        let error = checked(&source).unwrap_err();
        assert!(error.starts_with(diagnostic), "{body}: {error}");
    }
}

#[test]
fn explicit_mutable_scalar_bindings_lower_to_unique_ordered_mir_versions() {
    for body in [
        "let int x = 7; x = x + 1; return x;",
        "let x = 7; x += 1; return x;",
        "let x: int = 7; x -= 1; return x;",
        "mut int x = 7; x = x * 6; return x;",
        "mut x = 7; x += 1; x -= 2; return x;",
        "let x = 7; x += 1; const y = x; return y;",
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        let ast = frontend::parse(&source).expect("valid explicit mutable scalar syntax");
        let checked = mir::lower(ast).expect("SSA-style mutable lowering must typecheck");
        let unique = checked.functions[0].locals.iter()
            .map(|(name, _)| name.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), checked.functions[0].locals.len(),
            "{body}: internal SSA version names must be distinct");
        assert!(compile(&source).is_ok(), "{body}: native code emission");
    }
}

#[test]
fn mutable_scalar_reassignment_preserves_prior_reads_without_capture() {
    let source = concat!(
        "pub fnc main(): int { let x = 7; let a = x; x += 1; ",
        "let b = x; x = 42; return a + b + x; }"
    );
    let checked = mir::lower(frontend::parse(source).unwrap()).unwrap();
    let locals = &checked.functions[0].locals;
    assert_eq!(locals[0].0, "x");
    assert_eq!(locals[1].0, "a");
    assert_ne!(locals[2].0, "x");
    assert_ne!(locals[4].0, locals[2].0);
    assert!(matches!(&locals[1].1, frontend::Expr::Name(name) if name == "x"));
    assert!(matches!(&locals[3].1, frontend::Expr::Name(name) if name == &locals[2].0));
    assert!(compile(source).is_ok());
}

#[test]
fn mutable_scalar_violations_fail_closed() {
    for (body, class) in [
        ("let x = x + 1; return x;", "unknown_name"),
        ("let x = 7; let x = 8; return x;", "duplicate_declaration"),
        ("let x = 7; const x = 8; return x;", "duplicate_declaration"),
        ("let x = 7; missing = x; return x;", "unknown_name"),
        ("let x = 7; const y = x; y = 1; return y;", "immutable_binding"),
        ("let x = 7; x *= 2; return x;", "syntax_error"),
        ("return x;", "unknown_name"),
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        let error = checked(&source).unwrap_err();
        assert!(error.starts_with(class), "{body}: {error}");
        assert!(compile(&source).is_err(), "{body}: no native artifact");
    }
    let arg_write = "pub fnc f(int x): int { x = 2; return x; }";
    assert!(checked(arg_write).unwrap_err().starts_with("immutable_binding"));
}

#[test]
fn generated_ssa_names_cannot_capture_source_identifiers() {
    let source = concat!(
        "pub fnc main(): int { let x = 1; const __atto_scalar_rebind_0 = 3; ",
        "x = 42; return x + __atto_scalar_rebind_0; }"
    );
    let parsed = frontend::parse(source).unwrap();
    assert_ne!(parsed.functions[0].locals[2].name, "__atto_scalar_rebind_0");
    let typed = mir::lower(parsed).unwrap();
    assert_ne!(typed.functions[0].locals[2].0, typed.functions[0].locals[1].0);
    assert!(compile(source).is_ok());
}

#[test]
fn canonical_define_fnc_spellings_keep_export_and_scalar_checks() {
    for (source, exported) in [
        ("define pub fnc main(): int { return 42; }", true),
        ("pub define fnc main(): int { return 42; }", true),
        ("define fnc helper(): int { return 42; }", false),
    ] {
        let parsed = frontend::parse(source).expect("canonical fnc form");
        assert_eq!(parsed.functions[0].exported, exported);
        assert!(compile(source).is_ok(), "canonical definition should compile");
    }
    for source in [
        "define actor Worker as end",
        "define class Point as end",
        "define enum Status as end",
        "define define fnc main(): int { return 42; }",
    ] {
        assert!(checked(source).is_err(), "{source}: unsupported kinds fail closed");
    }
}

#[test]
fn typed_control_flow_and_actor_semantics_stay_fail_closed() {
    // Do not mistake source admission or a host-only model for compiled
    // Boolean control-flow, actor, or suspension execution.
    for source in [
        "pub fnc main(): int { if true then return 42; else return 0; fi }",
        "pub fnc main(): int { guard true { return 0; } return 42; }",
        "pub fnc main(): int { let x = 1; while x < 42 { x += 1; } return x; }",
        "pub fnc main(): int { match 1 { 1 => return 42; } }",
        "pub fnc main(): int { critical { return 42; } }",
        "suspends fnc main(): int { return 42; }",
        "define actor Worker as end",
    ] {
        assert!(checked(source).is_err(), "{source}: unsupported guest operation");
        assert!(compile(source).is_err(), "{source}: no partial native object");
    }
}
