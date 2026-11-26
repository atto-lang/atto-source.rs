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

fn compile_entry(entry: &str, source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_with_imports(entry, Path::new("src/main.atto"), |path| {
        if path == Path::new("src/types.atto") {
            Ok(source.to_owned())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unapproved import path"))
        }
    })
}

fn compile_with_imported(source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_entry(ENTRY, source)
}

#[test]
fn entry_namespace_precedes_imports_and_remains_visible_after_linker_injection() {
    let entry = r#"
use namespace application;
import * as types from "./types.atto";
pub fnc main() -> int { return types.int_value_bits(); }
"#;
    let wasm = compile_entry(entry, EXPLICIT_NAMESPACE)
        .expect("entry namespace must remain the first declaration after synthetic module linking");
    assert!(wasm.starts_with(&[0, 97, 115, 109]));
}

#[test]
fn rejects_late_and_duplicate_namespace_headers_in_importing_entry() {
    for (entry, code) in [
        (r#"import * as types from "./types.atto";
use namespace late;
pub fnc main() -> int { return types.int_value_bits(); }"#, "E_SYNTAX"),
        (r#"use namespace first;
use namespace second;
import * as types from "./types.atto";
pub fnc main() -> int { return types.int_value_bits(); }"#, "E_DUPLICATE"),
    ] {
        let error = compile_entry(entry, EXPLICIT_NAMESPACE)
            .expect_err("header placement must be checked before imports are stripped");
        assert_eq!(error.code, code, "{error}");
    }
}

#[test]
fn rejects_private_and_unbound_namespace_exports() {
    let private = "use namespace types; fnc int_value_bits() -> int { return 64; }";
    let error = compile_with_imported(private).expect_err("private import cannot cross source unit");
    assert_eq!(error.code, "E_PRIVATE");

    let missing = "use namespace types; pub fnc other_name() -> int { return 64; }";
    let error = compile_with_imported(missing).expect_err("missing exported function cannot be invented");
    assert_eq!(error.code, "E_UNBOUND");
}

#[test]
fn denies_entry_binding_collisions_path_traversal_and_internal_symbol_spoofing() {
    let collision = r#"
import * as type_lib from "./types.atto";
pub fnc type_lib() -> int { return 0; }
pub fnc main() -> int { return type_lib.int_value_bits(); }
"#;
    let error = compile_entry(collision, EXPLICIT_NAMESPACE)
        .expect_err("import cannot shadow local callable");
    assert_eq!(error.code, "E_DUPLICATE");

    let traversal = r#"
import * as types from "../../secret.atto";
pub fnc main() -> int { return 0; }
"#;
    let error = compile_entry(traversal, EXPLICIT_NAMESPACE)
        .expect_err("source-file imports cannot escape the logical package root");
    assert_eq!(error.code, "E_IMPORT_PATH");

    let forged = "use namespace types; pub fnc __atto_file_import_0() -> int { return 64; }";
    let error = compile_with_imported(forged)
        .expect_err("reserved generated module prefix cannot be supplied by guest source");
    assert_eq!(error.code, "E_RESERVED");
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

#[test]
fn contextual_types_preserves_keyword_and_header_identity() {
    let same = r#"use namespace types;
import * as types from "./types.atto";
pub fnc main() -> int { return types.int_value_bits(); }"#;
    assert!(compile_entry(same, EXPLICIT_NAMESPACE).unwrap().starts_with(b"\0asm"));
    let reserved = "pub fnc types() -> int { return 0; }";
    assert_eq!(atto_compiler::compile(reserved).unwrap_err().code, "E_RESERVED");
    assert!(atto_compiler::compile("pub fnc main() -> int { return types; }").is_err());
    assert!(atto_compiler::compile("pub fnc main() -> int { return types.missing(); }").is_err());
}
