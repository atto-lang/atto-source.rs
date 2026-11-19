//! Structural AST budget across composed, individually bounded parser expressions.
//! Runs actual compiler --check and Cranelift object emission; host-only assertions
//! are not presented as native guest execution.
//! This test is written for current main's 64 infix-operator and primary limits.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_compiler(source: &str, extension: &str, object_mode: bool) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "oresc-structural-depth-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("create isolated compiler fixture directory");
    let input = dir.join(format!("guest.{extension}"));
    let output = dir.join("guest.o");
    std::fs::write(&input, source).expect("write source fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_oresc"));
    if object_mode {
        command.arg(&input).arg(&output);
    } else {
        command.arg("--check").arg(&input);
    }
    let result = command.output().expect("execute actual compiler");
    if object_mode {
        assert_eq!(
            output.exists(),
            result.status.success(),
            "object publication must match compiler admission"
        );
    }
    std::fs::remove_dir_all(&dir).expect("remove fixture directory");
    result
}

fn scalar_chain(terms: usize) -> String {
    vec!["1"; terms].join("+")
}

// Parenthesized product plus an additive chain: each parser-local budget
// permits 64 operators, but the combined leftmost AST path can exceed 128.
fn composed_expression(product_terms: usize, additive_terms: usize) -> String {
    format!(
        "({}){}",
        vec!["1"; product_terms].join("*"),
        "+1".repeat(additive_terms - 1)
    )
}

fn source_for(expression: &str) -> String {
    format!("pub fnc main(): int {{ return {expression}; }}")
}

#[test]
fn valid_construction_still_compiles_and_produces_native_object() {
    for source in [
        source_for(&scalar_chain(24)),
        source_for(&composed_expression(64, 65)),
    ] {
        for extension in ["atto", "ores"] {
            for object_mode in [false, true] {
                let result = run_compiler(&source, extension, object_mode);
                assert_eq!(
                    result.status.code(),
                    Some(0),
                    "{extension}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
    }
}

#[test]
fn combined_ast_spine_is_rejected_before_native_output() {
    for source in [
        source_for(&composed_expression(65, 65)),
        format!(
            "pub fnc main(): int {{ const x: int = {}; return x; }}",
            composed_expression(65, 65)
        ),
        format!(
            "pub fnc main(): int {{ return helper({}); }} fnc helper(int x): int {{ return x; }}",
            composed_expression(64, 65)
        ),
    ] {
        for extension in ["atto", "ores"] {
            for object_mode in [false, true] {
                let result = run_compiler(&source, extension, object_mode);
                assert_eq!(result.status.code(), Some(1));
                assert!(result.stdout.is_empty(), "rejection emitted success output");
                let err = String::from_utf8_lossy(&result.stderr);
                assert!(
                    err.contains("resource_limit") && err.contains("expression AST depth"),
                    "unexpected diagnostic: {err}"
                );
            }
        }
    }
}

#[test]
fn existing_infix_limit_remains_a_separate_earlier_gate() {
    let source = source_for(&scalar_chain(130));
    for extension in ["atto", "ores"] {
        let result = run_compiler(&source, extension, true);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let err = String::from_utf8_lossy(&result.stderr);
        assert!(err.contains("resource_limit"));
        assert!(err.contains("additive expression exceeds scalar depth budget"));
    }
}
