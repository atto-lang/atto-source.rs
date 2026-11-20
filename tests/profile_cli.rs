use std::{fs, process::Command};

fn compiler() -> &'static str {
    env!("CARGO_BIN_EXE_oresc")
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("oreslang-source-rs-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn hard_rt_cli_fails_closed_without_wcet_evidence_and_publishes_nothing() {
    let dir = temp_dir("hardrt");
    let output = dir.join("answer.o");
    let source = dir.join("answer.ores");
    fs::write(&source, "pub fnc main(): int { return 42; }").expect("write exported root");
    let result = Command::new(compiler())
        .arg("--hard-rt")
        .arg(&source)
        .arg(&output)
        .output()
        .expect("run oresc");
    assert!(!result.status.success(), "--hard-rt unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("WCET/deadline evidence"), "{stderr}");
    assert!(
        !output.exists(),
        "failed hard-RT admission published an object"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn mutually_exclusive_profiles_are_rejected_before_publication() {
    let dir = temp_dir("profiles");
    let output = dir.join("answer.o");
    let result = Command::new(compiler())
        .args(["--release", "--strict-aot", "examples/answer.ores"])
        .arg(&output)
        .output()
        .expect("run oresc");
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("mutually exclusive"),
        "{:?}",
        result.stderr
    );
    assert!(!output.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn duplicate_freestanding_flag_is_rejected_before_publication() {
    let dir = temp_dir("freestanding");
    let output = dir.join("answer.o");
    let result = Command::new(compiler())
        .args(["--freestanding", "--freestanding", "examples/answer.ores"])
        .arg(&output)
        .output()
        .expect("run oresc");
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("duplicate --freestanding"),
        "{:?}",
        result.stderr
    );
    assert!(!output.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn release_cli_removes_unreachable_private_symbol() {
    let dir = temp_dir("tree-shake");
    let source = dir.join("tree_shake.ores");
    let object = dir.join("tree_shake.o");
    fs::write(
        &source,
        "pub fnc main(): int { return used(); }\nfnc used(): int { return 42; }\nfnc deliberately_dead_private(): int { return 99; }\n",
    )
    .expect("write source");

    let status = Command::new(compiler())
        .arg("--release")
        .arg(&source)
        .arg(&object)
        .status()
        .expect("run oresc");
    assert!(status.success(), "release compilation failed");

    let nm = Command::new("nm").arg(&object).output().expect("run nm");
    assert!(nm.status.success(), "nm failed");
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
        symbols
            .lines()
            .any(|line| line.split_whitespace().last() == Some("main")),
        "{symbols}"
    );
    assert!(
        !symbols.contains("deliberately_dead_private"),
        "unreachable private symbol survived release tree shaking: {symbols}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn strict_aot_retains_private_internal_calls_without_exporting_helper() {
    let dir = temp_dir("strict-aot-visibility");
    let source = dir.join("visibility.atto");
    let object = dir.join("visibility.o");
    let driver = dir.join("driver.c");
    let executable = dir.join("guest");
    fs::write(&source, include_str!("fixtures/private-linkage.atto")).unwrap();
    let compiler_run = Command::new(compiler())
        .arg("--strict-aot")
        .arg(&source)
        .arg(&object)
        .output()
        .expect("run strict AOT compiler");
    assert!(
        compiler_run.status.success(),
        "strict AOT rejected visibility fixture: {}",
        String::from_utf8_lossy(&compiler_run.stderr)
    );
    let nm = Command::new("nm")
        .args(["--extern-only", "--defined-only"])
        .arg(&object)
        .output()
        .expect("run nm on strict AOT object");
    assert!(nm.status.success(), "nm failed");
    let symbols = String::from_utf8_lossy(&nm.stdout);
    let names: Vec<&str> = symbols
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .collect();
    assert!(names.contains(&"answer"), "{symbols}");
    assert!(!names.contains(&"bump"), "{symbols}");
    fs::write(
        &driver,
        "extern long answer(long); int main(void) { return (int)answer(41); }\n",
    )
    .unwrap();
    let link = Command::new("cc")
        .arg(&driver)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("link strict-AOT native guest");
    assert!(
        link.status.success(),
        "strict AOT native link failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(&executable)
        .status()
        .expect("execute strict AOT native guest");
    assert_eq!(run.code(), Some(42));
    let _ = fs::remove_dir_all(dir);
}
