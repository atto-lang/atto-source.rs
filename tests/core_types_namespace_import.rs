//! Regression for source-file namespaces in the real Rust-Wasm import linker.
//! This compiles the *actual* imported direct functions; it is not a model of
//! arbitrary packages, general imports, or cross-backend guest execution.
use std::path::Path;
use atto_compiler::compile_with_imports;

const ENTRY: &str = r#"
import * as types from "./types.atto";
pub fnc main() -> int { return types.int_value_bits(); }
"#;
const EXPLICIT_NAMESPACE: &str = r#"
use namespace types;
pub fnc int_value_bits() -> int { return 64; }
"#;

fn compile_with_imported(source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_with_imports(ENTRY, Path::new("src/main.atto"), |path| {
        if path == Path::new("src/types.atto") {
            Ok(source.to_owned())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unapproved import path"))
        }
    })
}

#[test]
fn compiles_public_top_level_function_from_namespaced_source_file() {
    let wasm = compile_with_imported(EXPLICIT_NAMESPACE)
        .expect("file-level namespace must not become an invalid nested module header");
    assert!(wasm.starts_with(&[0, 97, 115, 109]), "actual emitted bytes must be Wasm");
}

#[test]
fn accepts_dotted_and_path_derived_file_namespaces_with_valid_logical_paths() {
    for source in [
        "use namespace core.types; pub fnc int_value_bits() -> int { return 64; }",
        "use default namespace; pub fnc int_value_bits() -> int { return 64; }",
        "use default namespace(1); pub fnc int_value_bits() -> int { return 64; }",
    ] {
        let wasm = compile_with_imported(source)
            .expect("header identity is validated against src/types.atto before linking");
        assert!(wasm.starts_with(&[0, 97, 115, 109]));
    }
}

#[test]
fn rejects_unresolvable_derived_namespace_and_imported_file_dependencies() {
    let bad_depth = "use default namespace(2); pub fnc int_value_bits() -> int { return 64; }";
    let err = compile_with_imported(bad_depth).expect_err("cannot infer two folders from src/types.atto");
    assert_eq!(err.code, "E_NAMESPACE_DEPTH");

    let unsupported_dependency = r#"
use namespace types;
import * as foreign from "./foreign.atto";
pub fnc int_value_bits() -> int { return 64; }
"#;
    let err = compile_with_imported(unsupported_dependency)
        .expect_err("one-hop function imports must reject nested dependency graphs");
    assert_eq!(err.code, "E_UNSUPPORTED");
}

#[test]
fn imported_file_namespace_is_not_optional_synthetic_module_shape() {
    let bare = "pub fnc int_value_bits() -> int { return 64; }";
    let wasm = compile_with_imported(bare).expect("legacy bare-file import remains valid");
    assert!(wasm.starts_with(&[0, 97, 115, 109]));
}

#[test]
fn rejects_duplicate_and_late_headers_in_imported_files() {
    for (bad, error_code) in [
        ("use namespace types; use namespace other; pub fnc int_value_bits() -> int { return 64; }", "E_DUPLICATE"),
        ("pub fnc int_value_bits() -> int { return 64; } use namespace late;", "E_SYNTAX"),
        ("use namespace types; pub fnc int_value_bits() -> int { return 64; } use namespace late;", "E_DUPLICATE"),
    ] {
        let err = compile_with_imported(bad).expect_err("namespace placement must fail before Wasm emission");
        assert_eq!(err.code, error_code, "{err}");
    }
}
