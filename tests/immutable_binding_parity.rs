//! Browser Wasm immutable binding regression. The legacy Cranelift-specific
//! AST/MIR/effect-export suite is preserved verbatim under tests/legacy;
//! this crate exposes atto_compiler::compile, not oreslang_source_rs.
use atto_compiler::compile;

fn source(binding: &str) -> String {
    format!("pub fnc main() -> int {{ {binding} return x + 2; }}")
}

#[test]
fn canonical_immutable_spellings_emit_identical_guest_bytes() {
    let forms = [
        "const x: int = 40;", "const int x = 40;", "const x = 40;",
        "int x = 40;", "val x = 40;", "val x: int = 40;",
    ];
    let expected = compile(&source(forms[0])).unwrap();
    assert!(expected.starts_with(b"\0asm"));
    for form in forms {
        assert_eq!(compile(&source(form)).unwrap(), expected, "{form}");
    }
}

#[test]
fn scalar_fixture_emits_guest() {
    let bytes = compile(include_str!("fixtures/immutable-bindings.atto")).unwrap();
    assert!(bytes.starts_with(b"\0asm"));
}

#[test]
fn duplicate_and_uninitialized_bindings_fail_closed() {
    for bindings in [
        "int x = 1; const x = 2;", "val x = 1; const int x = 2;",
        "const int x = 1; val x = 2;", "const x: int = 1; int x = 2;",
        "int x;", "const x;", "val x = ;", "const val x = 42;",
        "int x = missing;", "val x = y; const y = 42;", "const x = x + 1;",
        "const x: bool = true;",
    ] {
        assert!(compile(&source(bindings)).is_err(), "{bindings}");
    }
}

#[test]
fn immutable_binding_cannot_be_reassigned() {
    for form in ["int x = 40;", "const x = 40;", "val x = 40;"] {
        let src = format!("pub fnc main() -> int {{ {form} x = 41; return x; }}");
        assert!(compile(&src).is_err(), "{form}");
    }
}

#[test]
fn comment_boundaries_are_preserved() {
    for header in ["// /* block starts only inside the line comment\n",
                   "// /* unclosed pseudo-comment\n// more text\n",
                   "// */ and /* are just text\n",
                   "/* // not a line comment /* nested */ outer */ "] {
        assert!(compile(&format!("{header}pub fnc main() -> int {{ return 42; }}")).is_ok());
    }
    assert!(compile("// /* ignored\n/* real unclosed\npub fnc main() -> int { return 42; }").is_err());
}
