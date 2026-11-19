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
fn unsupported_mutable_spellings_stay_rejected() {
    for body in [
        "let int x = 7; return x;",
        "mut int x = 7; return x;",
        "int x = 7; x = 8; return x;",
        "val x = 7; val y = x + 1; y = 9; return y;",
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        assert!(compile(&source).is_err(), "{body}: no unimplemented mutable lowering");
    }
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
