//! Browser compiler CLI structural budgets, using the actual attoc binary.
//! Native Cranelift has its own tests/structural_ast_depth.rs suite.
//! Do not substitute a legacy native binary or claim object-file coverage.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn invoke(source: &str, extension: &str, emit: bool) -> Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("attoc-structural-depth-{}-{sequence}", std::process::id()));
    std::fs::create_dir(&dir).expect("create isolated fixture directory");
    let input = dir.join(format!("guest.{extension}"));
    let output = dir.join("guest.wasm");
    std::fs::write(&input, source).expect("write source");
    let mut command = Command::new(env!("CARGO_BIN_EXE_attoc"));
    if emit {
        command.arg(&input).arg("-o").arg(&output);
    } else {
        command.arg("--check").arg(&input);
    }
    let result = command.output().expect("execute attoc");
    if emit {
        assert_eq!(output.exists(), result.status.success(), "artifact publication must match admission");
        if result.status.success() {
            let wasm = std::fs::read(&output).expect("read guest");
            assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "expected Wasm module, not native object");
        }
    } else {
        assert!(!output.exists(), "check mode must not publish output");
    }
    std::fs::remove_dir_all(&dir).expect("remove fixture directory");
    result
}

fn source_for(expression: &str) -> String {
    format!("pub fnc main(): int {{ return {expression}; }}")
}
fn chain(terms: usize, op: &str) -> String {
    vec!["1"; terms].join(op)
}

#[test]
fn bounded_expressions_admit_browser_compiler_check_and_wasm() {
    for expression in [chain(24, "+"), format!("{}+({})", chain(24, "+"), chain(24, "*"))] {
        let source = source_for(&expression);
        for extension in ["atto", "ores"] {
            for emit in [false, true] {
                let result = invoke(&source, extension, emit);
                assert_eq!(result.status.code(), Some(0), "{extension}: {}", String::from_utf8_lossy(&result.stderr));
            }
        }
    }
}

#[test]
fn long_spines_and_nested_expressions_reject_without_artifact() {
    // Browser parser limits are MAX_TREE_HEIGHT=256 and MAX_DEPTH=128,
    // not the old native 64-operator profile.
    for expression in [
        chain(300, "+"),
        format!("{}1{}", "(".repeat(150), ")".repeat(150)),
        format!("{}+({})", chain(170, "+"), chain(140, "*")),
    ] {
        let source = source_for(&expression);
        for extension in ["atto", "ores"] {
            for emit in [false, true] {
                let result = invoke(&source, extension, emit);
                assert_eq!(result.status.code(), Some(1), "{extension}: {}", String::from_utf8_lossy(&result.stderr));
                assert!(result.stdout.is_empty(), "rejection emitted output");
                let err = String::from_utf8_lossy(&result.stderr);
                assert!(err.contains("E_NESTING"), "unexpected diagnostic: {err}");
            }
        }
    }
}
