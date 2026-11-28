//! Actual browser attoc structural expression budget and artifact publication.
//! The legacy native-compiler cases are retained in legacy/structural_ast_depth_cli.rs.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, check_only: bool) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("attoc-structural-depth-{}-{id}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let input = dir.join("guest.atto");
    let output = dir.join("guest.wasm");
    std::fs::write(&input, source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_attoc"));
    if check_only {
        command.arg("--check").arg(&input);
    } else {
        command.arg(&input).arg("-o").arg(&output);
    }
    let result = command.output().expect("actual browser compiler");
    let artifact = output.exists();
    if !check_only && result.status.success() {
        assert!(std::fs::read(&output).unwrap().starts_with(b"\0asm"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(artifact, !check_only && result.status.success(), "artifact admission mismatch");
    result
}
fn source(expr: &str) -> String {
    format!("pub fnc main(): int {{ return {expr}; }}")
}
#[test]
fn actual_attoc_accepts_bounded_and_denies_overbudget_expression_spines() {
    for check_only in [true, false] {
        let good = compile(&source(&vec!["1"; 42].join("+")), check_only);
        assert!(good.status.success(), "{}", String::from_utf8_lossy(&good.stderr));
        for expr in [
            vec!["1"; 300].join("+"),
            format!("({}){}", vec!["1"; 200].join("*"), "+1".repeat(200)),
        ] {
            let bad = compile(&source(&expr), check_only);
            assert_eq!(bad.status.code(), Some(1));
            assert!(bad.stdout.is_empty());
            assert!(String::from_utf8_lossy(&bad.stderr).contains("E_NESTING"), "{:?}", bad);
        }
    }
}
