//! Executable CLI regressions for byte-precise source admission across modes.
//! Keep these in Rust and exercise the actual compiler binary, not a mock parser.
use oreslang_source_rs::frontend::MAX_SOURCE_BYTES;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn scratch(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "oreslang-rs-source-bytes-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create isolated test directory");
    dir
}

fn source_with_exact_byte_count(bytes: usize) -> String {
    // The non-ASCII comment makes character counts differ from source bytes.
    // It must still be accepted at the limit and rejected one byte above.
    let prefix = "pub fnc main(): int { return 42; }\n//é";
    assert!(bytes >= prefix.len());
    format!("{prefix}{}", "x".repeat(bytes - prefix.len()))
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_oresc"))
        .args(args)
        .output()
        .expect("execute compiler CLI")
}

#[test]
fn exact_frontend_byte_limit_is_accepted_by_cli_and_native_emitter() {
    let dir = scratch("exact");
    let source = dir.join("at-limit.ores");
    let object = dir.join("at-limit.o");
    let contents = source_with_exact_byte_count(MAX_SOURCE_BYTES);
    assert_eq!(contents.len(), MAX_SOURCE_BYTES);
    fs::write(&source, contents).expect("write exact-limit source");

    let check = run(&["--check", source.to_str().unwrap()]);
    assert!(
        check.status.success(),
        "at-limit --check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("OK: 1 function(s)"));

    let compiled = run(&[
        "--strict-aot",
        source.to_str().unwrap(),
        object.to_str().unwrap(),
    ]);
    assert!(
        compiled.status.success(),
        "at-limit AOT failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(fs::metadata(&object).unwrap().len() > 0);
    fs::remove_dir_all(dir).expect("remove isolated test directory");
}

#[test]
fn byte_over_limit_is_rejected_before_object_or_facts_publication() {
    let dir = scratch("over");
    let source = dir.join("one-byte-over.ores");
    let object = dir.join("unexpected.o");
    let facts = dir.join("unexpected.json");
    let contents = source_with_exact_byte_count(MAX_SOURCE_BYTES + 1);
    assert_eq!(contents.len(), MAX_SOURCE_BYTES + 1);
    fs::write(&source, contents).expect("write over-limit source");

    let source_arg = source.to_str().unwrap();
    let object_arg = object.to_str().unwrap();
    let facts_arg = facts.to_str().unwrap();
    for arguments in [
        vec!["--check", source_arg],
        vec![source_arg, object_arg],
        vec!["--release", source_arg, object_arg],
        vec!["--strict-aot", source_arg, object_arg],
        vec!["--checked-i64", source_arg, object_arg],
        vec!["--compile-with-effects", source_arg, object_arg, facts_arg],
    ] {
        let result = run(&arguments);
        assert!(
            !result.status.success(),
            "oversized source unexpectedly passed {arguments:?}"
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("source too large"),
            "wrong rejection for {arguments:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!object.exists(), "rejected input published native output");
        assert!(!facts.exists(), "rejected input published effect facts");
    }
    fs::remove_dir_all(dir).expect("remove isolated test directory");
}
