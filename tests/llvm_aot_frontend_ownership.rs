//! Enforce a single owned Rust compiler frontend while experimenting with LLVM AOT.
//! This is a source-ownership gate, NOT a native guest execution test.
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

#[test]
fn llvm_adapter_depends_on_canonical_rust_frontend_not_vendor_copy() {
    let root = root();
    let adapter = root.join("experimental/llvm-aot");
    let manifest = fs::read_to_string(adapter.join("Cargo.toml")).expect("adapter Cargo.toml");
    assert!(manifest.contains("atto-compiler-browser = { path = \"../..\" }"),
        "the Rust LLVM adapter must depend on the canonical root compiler");
    for forbidden in [
        "frontend", "src/parser.rs", "src/check.rs", "src/lexer.rs",
        "src/ast.rs", "src/wasm.rs", "src/lib.rs",
    ] {
        assert!(!adapter.join(forbidden).exists(),
            "do not vendor a second Rust source-to-Wasm compiler: {forbidden}");
    }
    for required in ["src/main.rs", "src/target_native.rs", "runtime/actor_runtime.c"] {
        assert!(adapter.join(required).is_file(), "missing migrated AOT source: {required}");
    }
}

#[test]
fn llvm_adapter_does_not_enable_unreviewed_compiler_runtime_dependency() {
    let manifest = fs::read_to_string(root().join("experimental/llvm-aot/Cargo.toml"))
        .expect("adapter Cargo.toml");
    assert!(!manifest.contains("git = "), "do not use cross-repo credential-dependent Git dependencies");
    assert!(!manifest.contains("build = "), "do not introduce arbitrary build scripts");
    assert!(!manifest.contains("default-run = "), "experimental compiler is never the root default");
}
