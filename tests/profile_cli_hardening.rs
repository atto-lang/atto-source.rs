use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn compiler() -> &'static str {
    env!("CARGO_BIN_EXE_oresc")
}

fn scratch(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "oreslang-source-rs-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create scratch");
    dir
}

#[test]
#[cfg(target_os = "linux")]
fn strict_aot_removes_unreachable_private_symbol() {
    let dir = scratch("strict-aot");
    let source = dir.join("tree_shake.ores");
    let object = dir.join("tree_shake.o");
    fs::write(
        &source,
        "pub fnc main(): int { return used(); }\nfnc used(): int { return 42; }\nfnc deliberately_dead_private(): int { return 99; }\n",
    )
    .expect("write source");

    let status = Command::new(compiler())
        .arg("--strict-aot")
        .arg(&source)
        .arg(&object)
        .status()
        .expect("run oresc");
    assert!(status.success(), "strict-AOT compilation failed");

    let nm = Command::new("nm").arg(&object).output().expect("run nm");
    assert!(nm.status.success(), "nm failed");
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
        !symbols.contains("deliberately_dead_private"),
        "unreachable private symbol survived strict-AOT: {symbols}"
    );
    fs::remove_dir_all(dir).expect("remove scratch");
}

#[test]
fn oversized_profile_input_fails_before_object_publication() {
    let dir = scratch("oversized-profile");
    let source = dir.join("oversized.ores");
    let output = dir.join("oversized.o");
    fs::write(&source, vec![b'x'; 1_048_578]).expect("write oversized source");

    let result = Command::new(compiler())
        .arg("--release")
        .arg(&source)
        .arg(&output)
        .output()
        .expect("run oresc");
    assert!(
        !result.status.success(),
        "oversized source unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("source too large"), "{stderr}");
    assert!(!output.exists(), "oversized source published an object");
    fs::remove_dir_all(dir).expect("remove scratch");
}
