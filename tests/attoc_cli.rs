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


// A failed edit must not leave the previous valid Wasm file in place. Release
// automation and the playground must never confuse an old module with a new
// successfully compiled guest.
struct CliTemporaryDir(std::path::PathBuf);
impl CliTemporaryDir {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "atto-attoc-output-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for CliTemporaryDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn failed_recompile_and_unreadable_source_remove_previous_wasm() {
    let temp = CliTemporaryDir::new();
    let out = temp.0.join("compiled.wasm");
    let output = out.to_str().unwrap();
    let valid = "tests/fixtures/pass/01-answer.atto";
    let invalid = "tests/fixtures/fail/02-param-reassign.atto";
    let good = attoc(&[valid, "-o", output]);
    assert!(good.status.success(), "{}", String::from_utf8_lossy(&good.stderr));
    assert!(good.stdout.is_empty());
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"\\0asm\\x01\\0\\0\\0"));

    let bad = attoc(&[invalid, "-o", output]);
    assert_eq!(bad.status.code(), Some(1), "{}", String::from_utf8_lossy(&bad.stderr));
    assert!(bad.stdout.is_empty());
    assert!(!bad.stderr.is_empty());
    assert!(!out.exists(), "failed compile retained stale runnable guest");

    assert!(attoc(&[valid, "-o", output]).status.success());
    assert!(out.exists());
    let missing = attoc(&["tests/fixtures/this-source-is-missing.atto", "-o", output]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert!(!out.exists(), "read failure retained stale guest");
    assert!(attoc(&[valid, "-o", output]).status.success());
    assert_eq!(std::fs::read(&out).unwrap(), bytes, "rebuild must be deterministic");
}

#[test]
fn compiler_output_cannot_overwrite_its_input_file_or_aliases() {
    let temp = CliTemporaryDir::new();
    let input = temp.0.join("source.atto");
    let original = std::fs::read("tests/fixtures/pass/01-answer.atto").unwrap();
    std::fs::write(&input, &original).unwrap();
    let path = input.to_str().unwrap();
    let same = attoc(&[path, "-o", path]);
    assert_eq!(same.status.code(), Some(2));
    assert!(same.stdout.is_empty());
    assert_eq!(std::fs::read(&input).unwrap(), original);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let alias = temp.0.join("source-alias.wasm");
        symlink(&input, &alias).unwrap();
        let result = attoc(&[path, "-o", alias.to_str().unwrap()]);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert!(alias.is_symlink(), "alias must not be unlinked by failed output guard");

        let hard_alias = temp.0.join("hardlink.wasm");
        std::fs::hard_link(&input, &hard_alias).unwrap();
        let result = attoc(&[path, "-o", hard_alias.to_str().unwrap()]);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(std::fs::read(&hard_alias).unwrap(), original);
        assert_eq!(std::fs::read(&input).unwrap(), original);
    }
}

#[test]
fn preexisting_nonsource_artifacts_are_replaced_not_appended() {
    let temp = CliTemporaryDir::new();
    let target = temp.0.join("old.wasm");
    std::fs::write(&target, b"NOT A COMPILED WASM GUEST").unwrap();
    let result = attoc(&["tests/fixtures/pass/01-answer.atto", "-o", target.to_str().unwrap()]);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let bytes = std::fs::read(&target).unwrap();
    assert!(bytes.starts_with(b"\\0asm\\x01\\0\\0\\0"));
    assert!(!bytes.windows(b"NOT A COMPILED WASM GUEST".len())
        .any(|w| w == b"NOT A COMPILED WASM GUEST"));
}
