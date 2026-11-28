//! Browser compiler structural and nesting budgets through the actual attoc CLI.
//! Original Cranelift native object suite preserved in verification/legacy-native-aot.
//! These checks establish browser artifact admission, not native object emission.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_compiler(source: &str, check: bool) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "attoc-structural-depth-{}-{id}", std::process::id()
    ));
    std::fs::create_dir(&dir).expect("create isolated fixture directory");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let _cleanup = Cleanup(dir.clone());
    let input = dir.join("guest.atto");
    let output = dir.join("guest.wasm");
    std::fs::write(&input, source).expect("write source fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_attoc"));
    if check {
        command.arg("--check").arg(&input);
    } else {
        command.arg(&input).arg("-o").arg(&output);
    }
    let result = command.output().expect("execute actual compiler");
    assert!(result.status.code().is_some(), "compiler must not terminate by signal");
    if check {
        assert!(!output.exists(), "check never publishes a Wasm artifact");
    } else if result.status.success() {
        assert!(std::fs::read(&output).unwrap().starts_with(b"\0asm"));
    } else {
        assert!(!output.exists(), "rejected source must not publish Wasm");
    }
    result
}

fn scalar_chain(terms: usize) -> String { vec!["1"; terms].join("+") }
fn composed_expression(product: usize, sum: usize) -> String {
    format!("({}){}", vec!["1"; product].join("*"), "+1".repeat(sum - 1))
}
fn source_for(expression: &str) -> String {
    format!("pub fnc main(): int {{ return {expression}; }}")
}

#[test]
fn valid_constructed_spines_compile_and_publish_wasm() {
    for source in [
        source_for(&scalar_chain(24)),
        source_for(&scalar_chain(250)),
        source_for(&composed_expression(64, 65)),
    ] {
        for check in [false, true] {
            let r = run_compiler(&source, check);
            assert_eq!(r.status.code(), Some(0), "{}", String::from_utf8_lossy(&r.stderr));
        }
    }
}

#[test]
fn excessive_operator_spines_are_rejected_without_artifact() {
    let expr = scalar_chain(300);
    for source in [
        source_for(&expr),
        format!("pub fnc main(): int {{ const x: int = {expr}; return x; }}"),
        format!("pub fnc main(): int {{ return helper({expr}); }} fnc helper(int x): int {{ return x; }}"),
    ] {
        for check in [false, true] {
            let r = run_compiler(&source, check);
            assert_eq!(r.status.code(), Some(1));
            assert!(r.stdout.is_empty());
            assert!(String::from_utf8_lossy(&r.stderr).contains("E_NESTING"));
        }
    }
}

#[test]
fn nested_primary_budget_is_independent_of_long_infix_spines() {
    let expr = format!("{}1{}", "(".repeat(150), ")".repeat(150));
    for check in [false, true] {
        let r = run_compiler(&source_for(&expr), check);
        assert_eq!(r.status.code(), Some(1));
        assert!(r.stdout.is_empty());
        assert!(String::from_utf8_lossy(&r.stderr).contains("E_NESTING"));
    }
}
