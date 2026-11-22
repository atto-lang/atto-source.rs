//! Browser Wasm compilation cannot silently expose a native UDP socket.
//! These tests execute the real attoc CLI, not the Node source-model harness.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Input(std::path::PathBuf);
impl Input {
    fn new(source: &str) -> Self {
        for _ in 0..64 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-udp-failclosed-{}-{id}.atto", std::process::id()));
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(source.as_bytes()).expect("write source");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create UDP test source: {e}"),
            }
        }
        panic!("cannot reserve UDP test source path");
    }
}
impl Drop for Input { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }

#[test]
fn browser_compiler_rejects_unimplemented_raw_udp_host_calls() {
    // Guard against a fake green result from a CLI that rejects ALL input.
    let baseline = Command::new(env!("CARGO_BIN_EXE_attoc"))
        .arg("--check").arg("tests/fixtures/pass/01-answer.atto")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output().expect("launch real compiler positive control");
    assert!(baseline.status.success(), "valid scalar baseline must compile");
    assert!(baseline.stderr.is_empty());
    let source = Input::new(
        "fnc main(): int { return network.bind_udp(\"127.0.0.1\", 0); }");
    for flags in [vec!["--check"], vec![]] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_attoc"));
        command.args(flags).arg(&source.0);
        let output = command.output().expect("run real attoc compiler");
        assert!(!output.status.success(), "browser compiler accepted raw UDP host API");
        assert!(output.stdout.is_empty(),
            "unsupported browser raw UDP must never emit a wasm module or PASS");
        assert!(!output.stderr.is_empty(), "unsupported raw UDP needs a diagnostic");
    }
}

#[test]
fn browser_compiler_does_not_treat_udp_policy_source_model_as_guest_proof() {
    // This checks the separate transport capability boundary only. The .atto
    // policy/queue fixtures still need genuine compiled-guest conformance.
    let source = Input::new(
        "fnc main(): int { return std.net.udp.bind(0); }");
    let output = Command::new(env!("CARGO_BIN_EXE_attoc"))
        .arg("--check").arg(&source.0).output().expect("run attoc");
    assert!(!output.status.success(), "unknown std.net.udp socket binding accepted");
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
