//! Native executable parity against the LLVM shared v1 portable i64 corpus.
//! Each .atto file is byte-identical to the original LLVM .ores fixture.
#![cfg(target_os = "linux")]

use oreslang_source_rs::compile;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TemporaryBuild(PathBuf);

impl TemporaryBuild {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oresc-shared-scalar-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create native guest build directory");
        Self(path)
    }
}

impl Drop for TemporaryBuild {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_native_guest(source: &str) {
    let temp = TemporaryBuild::new();
    let object = temp.0.join("guest.o");
    let executable = temp.0.join("guest");
    let bytes = compile(source)
        .unwrap_or_else(|error| panic!("compilation failed for {source:?}: {error}"));
    assert!(bytes.starts_with(b"\x7fELF"), "expected ELF native object");
    fs::write(&object, bytes).expect("write native object");
    let link = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("Linux guest conformance needs C linker");
    assert!(
        link.status.success(),
        "guest link failed for {source:?}: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    let guest = Command::new(&executable)
        .output()
        .expect("execute linked native guest");
    assert_eq!(
        guest.status.code(),
        Some(42),
        "guest returned incorrect exit status for {source:?}; stderr={}",
        String::from_utf8_lossy(&guest.stderr)
    );
}

#[test]
fn llvm_shared_scalar_corpus_runs_through_rust_cranelift_native_linker() {
    for source in [
        include_str!("fixtures/oresllm_scalar_v1/oresllm-precedence.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-scalar-gradient.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-signed-product.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-grouped-negative.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-nested-precedence.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-max-minus-one.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-zero-product.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-nested-arithmetic.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-subtract-self.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-zero-add.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-signed-subtract.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-left-associative.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-zero-times-max.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-over-2pow53.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-min-max-cancel.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-near-square.atto"),
        include_str!("fixtures/oresllm_scalar_v1/oresllm-over53-cancel.atto"),
    ] {
        run_native_guest(source);
    }
}
