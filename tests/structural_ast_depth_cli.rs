//! Actual browser compiler CLI structural resource gates.
//! The native Cranelift host test is retained in legacy-host; this crate
//! builds attoc, which emits Wasm, and uses the browser parser's limits.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_compiler(source: &str, extension: &str, emit: bool) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("attoc-depth-{}-{id}", std::process::id()));
    std::fs::create_dir(&dir).expect("create isolated compiler fixture directory");
    let input = dir.join(format!("guest.{extension}"));
    let output = dir.join("guest.wasm");
    std::fs::write(&input, source).expect("write source fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_attoc"));
    if emit { command.arg(&input).arg("-o").arg(&output); }
    else { command.arg("--check").arg(&input); }
    let result = command.output().expect("execute actual browser compiler");
    if emit {
        assert_eq!(output.exists(), result.status.success(), "artifact publication matches admission");
        if result.status.success() {
            assert!(std::fs::read(&output).unwrap().starts_with(b"\0asm\x01\0\0\0"));
        }
    }
    std::fs::remove_dir_all(&dir).expect("remove fixture directory");
    result
}

fn scalar_chain(terms: usize) -> String { vec!["1"; terms].join("+") }
fn composed_expression(product_terms: usize, additive_terms: usize) -> String {
    format!("({}){}", vec!["1"; product_terms].join("*"), "+1".repeat(additive_terms - 1))
}
fn source_for(expression: &str) -> String {
    format!("pub fnc main(): int {{ return {expression}; }}")
}

#[test]
fn valid_construction_compiles_and_produces_wasm() {
    for source in [source_for(&scalar_chain(24)), source_for(&composed_expression(64,65))] {
        for extension in ["atto", "ores"] {
            for emit in [false, true] {
                let result = run_compiler(&source, extension, emit);
                assert_eq!(result.status.code(), Some(0), "{}", String::from_utf8_lossy(&result.stderr));
            }
        }
    }
}

#[test]
fn oversized_nested_and_composed_expressions_reject_without_artifact() {
    let nested = format!("{}1{}", "(".repeat(140), ")".repeat(140));
    let over_chain = composed_expression(270, 65);
    for source in [
        source_for(&nested),
        source_for(&over_chain),
        format!("pub fnc main(): int {{ const x: int = {over_chain}; return x; }}"),
        format!("pub fnc main(): int {{ return helper({over_chain}); }} fnc helper(int x): int {{ return x; }}"),
    ] {
        for extension in ["atto", "ores"] {
            for emit in [false, true] {
                let result = run_compiler(&source, extension, emit);
                assert_eq!(result.status.code(), Some(1), "{}", String::from_utf8_lossy(&result.stderr));
                assert!(result.stdout.is_empty(), "rejection emitted success output");
                let err = String::from_utf8_lossy(&result.stderr);
                assert!(err.contains("E_NESTING"), "unexpected diagnostic: {err}");
            }
        }
    }
}

#[test]
fn flat_operator_chain_budget_remains_separate_from_parentheses() {
    let source = source_for(&scalar_chain(270));
    for extension in ["atto", "ores"] {
        let result = run_compiler(&source, extension, true);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let err = String::from_utf8_lossy(&result.stderr);
        assert!(err.contains("E_NESTING"), "unexpected diagnostic: {err}");
    }
}
