//! CLI admission and output-publication regressions for opt-in checked i64.
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TemporaryDir(PathBuf);

impl TemporaryDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oresc-checked-i64-cli-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TemporaryDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compiler() -> &'static str {
    env!("CARGO_BIN_EXE_oresc")
}

#[test]
fn checked_arithmetic_refuses_incomplete_safety_and_effect_exports() {
    let dir = TemporaryDir::new();
    let source = dir.0.join("guest.atto");
    fs::write(&source,
        "pub fnc main(): int { return 9223372036854775807+1; }",
    )
    .expect("create source");
    for flag in [
        "--effects",
        "--safety-ir",
        "--static-facts",
        "--cfg-ir",
        "--check",
    ] {
        let result = Command::new(compiler())
            .arg("--checked-i64")
            .arg(flag)
            .arg(&source)
            .output()
            .expect("run checked-i64 export refusal");
        assert!(!result.status.success(), "{flag} unexpectedly succeeded");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("not yet modeled"),
            "{flag}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let flow = Command::new(compiler())
        .args(["--checked-i64", "--flow", "8"])
        .arg(&source)
        .output()
        .expect("run checked-i64 flow refusal");
    assert!(!flow.status.success());
    let object = dir.0.join("should-not-exist.o");
    let effects = dir.0.join("should-not-exist.json");
    let paired = Command::new(compiler())
        .args(["--checked-i64", "--compile-with-effects"])
        .arg(&source)
        .arg(&object)
        .arg(&effects)
        .output()
        .expect("run checked-i64 paired-export refusal");
    assert!(!paired.status.success());
    assert!(!object.exists() && !effects.exists(), "incomplete facts published");
}

#[test]
fn duplicate_checked_flag_fails_before_output_publication() {
    let dir = TemporaryDir::new();
    let object = dir.0.join("unexpected.o");
    let result = Command::new(compiler())
        .args(["--checked-i64", "--checked-i64"])
        .arg("examples/answer.ores")
        .arg(&object)
        .output()
        .expect("execute duplicate flag test");
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("duplicate --checked-i64")
    );
    assert!(!object.exists());
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn checked_strict_aot_cli_compiles_and_executes_native_guest() {
    let dir = TemporaryDir::new();
    let source = dir.0.join("positive.atto");
    let object = dir.0.join("positive.o");
    let executable = dir.0.join("positive");
    fs::write(
        &source,
        include_str!("fixtures/checked_i64_v1/signed.atto"),
    )
    .expect("write positive fixture");
    let result = Command::new(compiler())
        .args(["--strict-aot", "--checked-i64"])
        .arg(&source)
        .arg(&object)
        .output()
        .expect("run strict AOT checked compiler");
    assert!(
        result.status.success(),
        "strict checked AOT compile failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let link = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("link strict checked AOT executable");
    assert!(
        link.status.success(),
        "native link failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    let guest = Command::new(&executable)
        .status()
        .expect("run strict checked AOT guest");
    assert_eq!(guest.code(), Some(42));
}
