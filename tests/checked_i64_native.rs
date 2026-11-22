//! Opt-in Rust/Cranelift checked-i64 native guest execution gate.
//! Reference: LLVM tests/rust-harness/src/checked_i64.rs.
//! A native trap is NOT a typed AttoLang Error or an actor panic notice.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use oreslang_source_rs::compile_checked_i64;
use std::{
    fs,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, ExitStatus},
    time::{SystemTime, UNIX_EPOCH},
};

struct GuestBuild(PathBuf);

impl GuestBuild {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before UNIX epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "oreslang-checked-i64-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create native checked-i64 test directory");
        Self(dir)
    }
}

impl Drop for GuestBuild {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compile_link_execute(source: &str) -> ExitStatus {
    let dir = GuestBuild::new();
    let object = dir.0.join("guest.o");
    let executable = dir.0.join("guest");
    let bytes = compile_checked_i64(source)
        .unwrap_or_else(|error| panic!("checked native compile failed: {error}"));
    assert!(bytes.starts_with(b"\x7fELF"), "expected real ELF object");
    fs::write(&object, bytes).expect("write compiled object");
    let link = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("Linux C linker must be available");
    assert!(
        link.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    Command::new(&executable)
        .current_dir(&dir.0)
        .status()
        .expect("execute compiled native guest")
}

#[test]
fn checked_i64_portable_positive_native_guest_matrix() {
    for (source, expected) in [
        (include_str!("fixtures/checked_i64_v1/precedence.atto"), 14),
        (include_str!("fixtures/checked_i64_v1/parentheses.atto"), 20),
        (include_str!("fixtures/checked_i64_v1/signed.atto"), 42),
        (include_str!("fixtures/checked_i64_v1/near-max.atto"), 1),
        (include_str!("fixtures/checked_i64_v1/near-min.atto"), 1),
        (include_str!("fixtures/checked_i64_v1/negate.atto"), 42),
        (include_str!("fixtures/checked_i64_v1/dynamic.atto"), 42),
        (
            include_str!("fixtures/checked_i64_v1/zero-times-max.atto"),
            42,
        ),
    ] {
        let observed = compile_link_execute(source);
        assert_eq!(observed.code(), Some(expected), "{source:?}: {observed:?}");
    }
}

#[test]
fn checked_i64_overflow_must_trap_instead_of_wrapping() {
    for source in [
        include_str!("fixtures/checked_i64_v1/add.atto"),
        include_str!("fixtures/checked_i64_v1/subtract.atto"),
        include_str!("fixtures/checked_i64_v1/multiply.atto"),
        include_str!("fixtures/checked_i64_v1/dynamic-add.atto"),
        include_str!("fixtures/checked_i64_v1/dynamic-subtract.atto"),
        include_str!("fixtures/checked_i64_v1/dynamic-multiply.atto"),
        include_str!("fixtures/checked_i64_v1/dynamic-negate.atto"),
    ] {
        let observed = compile_link_execute(source);
        // LLVM's checked-i64 reference expects SIGILL/UD2 on Linux x86-64.
        // A normal exit or an unrelated signal is not accepted.
        assert_eq!(observed.signal(), Some(4), "{source:?}: {observed:?}");
    }
}

#[test]
fn negated_minimum_literal_fails_before_native_emission() {
    let source = "pub fnc main(): int { return -(-9223372036854775808); }";
    let error = compile_checked_i64(source).unwrap_err();
    assert!(error.starts_with("integer_overflow"), "{error}");
}
