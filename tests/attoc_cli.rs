//! CLI smoke tests exercise the actual attoc binary and shared .atto fixtures.
use std::process::Command;

fn attoc(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_attoc"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("attoc must be launchable")
}

#[test]
fn check_only_accepts_valid_atto_without_emitting_wasm() {
    let output = attoc(&["--check", "tests/fixtures/pass/01-answer.atto"]);
    assert!(output.status.success(), "{:?}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty(), "--check must not emit binary output");
    assert!(output.stderr.is_empty());
}

#[test]
fn check_only_preserves_compiler_diagnostics_and_exit_codes() {
    let output = attoc(&["--check", "tests/fixtures/fail/02-param-reassign.atto"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty(), "failed checking must have diagnostics");
}

#[test]
fn cli_help_and_bad_arguments_do_not_generate_artifacts() {
    for flag in ["--help", "-h"] {
        let output = attoc(&[flag]);
        assert!(output.status.success());
        assert!(output.stdout.starts_with(b"usage: attoc"));
    }
    let output = attoc(&["--check", "tests/fixtures/pass/01-answer.atto", "-o", "out.wasm"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn default_compile_still_emits_wasm_to_stdout() {
    let output = attoc(&["tests/fixtures/pass/01-answer.atto"]);
    assert!(output.status.success(), "{:?}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.starts_with(b"\0asm\x01\0\0\0"));
}
