use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "oresc-publication-{label}-{}-{nanos}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).expect("create scratch");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source(path: &Path) {
    fs::write(path, "pub fnc main(): int { return 42; }\n").expect("write source");
}

fn run(source: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_oresc"))
        .arg(source)
        .arg(output)
        .output()
        .expect("invoke compiler")
}

fn no_temporary_files(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read output dir") {
        let name = entry.expect("entry").file_name();
        assert!(
            !name.to_string_lossy().starts_with(".oresc-"),
            "staging file leaked: {name:?}"
        );
    }
}

#[test]
fn existing_hardlink_to_source_is_safely_replaced_without_truncation() {
    let dir = Scratch::new("hardlink");
    let input = dir.path("main.ores");
    let output = dir.path("main.o");
    source(&input);
    let original = fs::read(&input).unwrap();
    fs::hard_link(&input, &output).expect("hardlink source");
    let result = run(&input, &output);
    assert!(
        result.status.success(),
        "hardlinked destination should be safely replaced: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(&input).unwrap(), original, "source was modified");
    let emitted = fs::read(&output).unwrap();
    assert!(!emitted.is_empty() && emitted != original);
    no_temporary_files(&dir.0);
}

#[test]
fn successful_replacement_does_not_leave_staging_files() {
    let dir = Scratch::new("replace");
    let input = dir.path("main.ores");
    let output = dir.path("main.o");
    source(&input);
    fs::write(&output, b"previous complete object").unwrap();
    let result = run(&input, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_ne!(fs::read(&output).unwrap().as_slice(), b"previous complete object");
    no_temporary_files(&dir.0);
}

#[test]
#[cfg(unix)]
fn final_symlink_cannot_redirect_native_output() {
    use std::os::unix::fs::symlink;
    let dir = Scratch::new("symlink");
    let input = dir.path("main.ores");
    let victim = dir.path("victim.txt");
    let output = dir.path("main.o");
    source(&input);
    fs::write(&victim, b"must remain intact").unwrap();
    symlink(&victim, &output).unwrap();
    let result = run(&input, &output);
    assert!(!result.status.success(), "symlinked output was accepted");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("refusing symlinked compiler output")
    );
    assert_eq!(fs::read(&victim).unwrap(), b"must remain intact");
    assert!(fs::symlink_metadata(&output).unwrap().file_type().is_symlink());
    no_temporary_files(&dir.0);
}

#[test]
#[cfg(unix)]
fn dangling_symlink_is_rejected_without_creating_target() {
    use std::os::unix::fs::symlink;
    let dir = Scratch::new("dangling");
    let input = dir.path("main.ores");
    let target = dir.path("missing.txt");
    let output = dir.path("main.o");
    source(&input);
    symlink(&target, &output).unwrap();
    let result = run(&input, &output);
    assert!(!result.status.success());
    assert!(!target.exists());
    no_temporary_files(&dir.0);
}

#[test]
fn output_directory_is_not_replaced_by_an_artifact() {
    let dir = Scratch::new("directory");
    let input = dir.path("main.ores");
    let output = dir.path("object.o");
    source(&input);
    fs::create_dir(&output).unwrap();
    let result = run(&input, &output);
    assert!(!result.status.success());
    assert!(output.is_dir());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("compiler output must be a regular file")
    );
    no_temporary_files(&dir.0);
}

#[test]
fn rejected_compilation_keeps_existing_object_unchanged() {
    let dir = Scratch::new("rejected");
    let input = dir.path("invalid.ores");
    let output = dir.path("previous.o");
    fs::write(&input, "pub fnc main(): int { return not_declared; }\n").unwrap();
    fs::write(&output, b"existing object").unwrap();
    let result = run(&input, &output);
    assert!(!result.status.success());
    assert_eq!(fs::read(&output).unwrap(), b"existing object");
    no_temporary_files(&dir.0);
}

#[test]
#[cfg(unix)]
fn paired_effect_symlink_fails_before_publishing_object() {
    use std::os::unix::fs::symlink;
    let dir = Scratch::new("paired");
    let input = dir.path("main.ores");
    let object = dir.path("main.o");
    let effects = dir.path("effects.json");
    let victim = dir.path("victim.json");
    source(&input);
    fs::write(&victim, b"do not overwrite").unwrap();
    symlink(&victim, &effects).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_oresc"))
        .args(["--compile-with-effects"])
        .arg(&input)
        .arg(&object)
        .arg(&effects)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!object.exists());
    assert_eq!(fs::read(&victim).unwrap(), b"do not overwrite");
    no_temporary_files(&dir.0);
}
