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
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));

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
fn compiler_refuses_source_extension_output_before_removing_existing_files() {
    let temp = CliTemporaryDir::new();
    let original = b"// unrelated Atto module must survive -o\npub fnc unrelated(): int { return 11; }\n";
    for suffix in ["atto", "ATTO"] {
        let output = temp.0.join(format!("unrelated.{suffix}"));
        std::fs::write(&output, original).unwrap();
        let result = attoc(&[
            "tests/fixtures/pass/01-answer.atto", "-o", output.to_str().unwrap(),
        ]);
        assert_eq!(result.status.code(), Some(2), "{suffix}: {:?}", result.status);
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr)
            .contains("Wasm output cannot use a .atto source extension"));
        assert_eq!(std::fs::read(&output).unwrap().as_slice(), &original[..],
            "{suffix}: compiler deleted or changed unrelated source");
    }
    let not_created = temp.0.join("new.atto");
    let result = attoc(&[
        "tests/fixtures/pass/01-answer.atto", "-o", not_created.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(!not_created.exists(), "invalid output extension created a source file");
}

#[test]
fn preexisting_nonsource_artifacts_are_replaced_not_appended() {
    let temp = CliTemporaryDir::new();
    let target = temp.0.join("old.wasm");
    std::fs::write(&target, b"NOT A COMPILED WASM GUEST").unwrap();
    let result = attoc(&["tests/fixtures/pass/01-answer.atto", "-o", target.to_str().unwrap()]);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let bytes = std::fs::read(&target).unwrap();
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert!(!bytes.windows(b"NOT A COMPILED WASM GUEST".len())
        .any(|w| w == b"NOT A COMPILED WASM GUEST"));
}

#[test]
fn oversized_entry_source_fails_before_publishing_a_guest() {
    let temp = CliTemporaryDir::new();
    let huge = temp.0.join("huge.atto");
    let output = temp.0.join("compiled.wasm");
    // A bounded 64 KiB source reader must not allocate the whole input first.
    // The guard should classify this as a compiler source-limit rejection,
    // and invalidate any output from an earlier successful build.
    std::fs::write(&huge, vec![b'x'; atto_compiler::SOURCE_CAP + 1]).unwrap();
    let valid = "tests/fixtures/pass/01-answer.atto";
    assert!(attoc(&[valid, "-o", output.to_str().unwrap()]).status.success());
    assert!(output.exists());
    let denied = attoc(&[huge.to_str().unwrap(), "-o", output.to_str().unwrap()]);
    assert_eq!(denied.status.code(), Some(1), "{:?}", denied.status);
    assert!(denied.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("E_SOURCE_LIMIT"),
        "expected a source limit diagnostic: {}",
        String::from_utf8_lossy(&denied.stderr)
    );
    assert!(!output.exists(), "stale executable survived oversized input");

    let check_only = attoc(&["--check", huge.to_str().unwrap()]);
    assert_eq!(check_only.status.code(), Some(1));
    assert!(check_only.stdout.is_empty());
    assert!(String::from_utf8_lossy(&check_only.stderr).contains("E_SOURCE_LIMIT"));
}

#[test]
fn exactly_source_cap_input_is_not_rejected_by_bounded_reader() {
    let temp = CliTemporaryDir::new();
    let source = temp.0.join("limit.atto");
    let prefix = b"pub fnc main(): int { return 42; }\n";
    let mut bytes = prefix.to_vec();
    bytes.resize(atto_compiler::SOURCE_CAP, b' ');
    std::fs::write(&source, bytes).unwrap();
    let out = temp.0.join("limit.wasm");
    let result = attoc(&[source.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(result.status.success(), "exact source cap rejected: {}",
        String::from_utf8_lossy(&result.stderr));
    assert!(std::fs::read(out).unwrap().starts_with(b"\\0asm\\x01\\0\\0\\0"));
}


#[cfg(target_os = "linux")]
#[test]
fn fifo_input_cannot_block_compilation_or_preserve_a_stale_guest() {
    // GNU coreutils are available in the Ubuntu CI runner. A missing reader
    // preflight would block on File::open(FIFO), so enforce a hard timeout.
    let temp = CliTemporaryDir::new();
    let fifo = temp.0.join("incoming.atto");
    let wasm = temp.0.join("old.wasm");
    let created = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(created.success(), "could not create test FIFO");
    std::fs::write(&wasm, b"STALE_GUEST_MUST_NOT_SURVIVE").unwrap();
    let result = Command::new("timeout")
        .args(["4s", env!("CARGO_BIN_EXE_attoc")])
        .arg(fifo.to_str().unwrap())
        .arg("-o")
        .arg(wasm.to_str().unwrap())
        .output()
        .expect("GNU timeout must be installed on the Linux CI runner");
    assert_eq!(result.status.code(), Some(2),
        "non-regular source was accepted or blocked past timeout: {}",
        String::from_utf8_lossy(&result.stderr));
    assert!(result.stdout.is_empty());
    assert!(!wasm.exists(), "old Wasm guest survived rejected FIFO input");

    use std::os::unix::fs::symlink;
    let alias = temp.0.join("incoming-alias.atto");
    symlink(&fifo, &alias).unwrap();
    let aliased = Command::new("timeout")
        .args(["4s", env!("CARGO_BIN_EXE_attoc")])
        .arg(alias.to_str().unwrap())
        .output()
        .expect("timeout must be installed");
    assert_eq!(aliased.status.code(), Some(2),
        "source symlink to FIFO caused blocking open");
    assert!(aliased.stdout.is_empty());
}


#[cfg(target_os = "linux")]
#[test]
fn package_source_and_manifest_fifo_fail_without_blocking_open() {
    // LockedRoot::read uses openat() and must reject a FIFO *before* attempting
    // to read from it. GNU timeout turns a regression into a bounded failure
    // instead of hanging the entire CI workflow indefinitely.
    let temp = CliTemporaryDir::new();
    let root = temp.0.to_str().unwrap();
    let manifest = temp.0.join(".atto.pkg.toml");
    let main = temp.0.join("main.atto");
    let output = temp.0.join("compiled.wasm");
    let valid_manifest = "[package]\nname=\"fifo_guard\"\nversion=\"1.0.0\"\n";
    std::fs::write(&manifest, valid_manifest).unwrap();
    std::fs::write(&main, "pub fnc main(): int { return 42; }\n").unwrap();
    let valid = attoc(&["--package-root", root, "--entry", "main.atto", "-o", output.to_str().unwrap()]);
    assert!(valid.status.success(), "{}", String::from_utf8_lossy(&valid.stderr));
    assert!(output.exists());

    std::fs::remove_file(&main).unwrap();
    assert!(Command::new("mkfifo").arg(&main).status().unwrap().success());
    let denied = Command::new("timeout")
        .args(["4s", env!("CARGO_BIN_EXE_attoc"), "--package-root", root,
               "--entry", "main.atto", "-o", output.to_str().unwrap()])
        .output().unwrap();
    assert_eq!(denied.status.code(), Some(1),
        "openat FIFO must fail before blocking: {}",
        String::from_utf8_lossy(&denied.stderr));
    assert!(denied.stdout.is_empty());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("E_IMPORT_PATH"));
    assert!(!output.exists(), "failed package build retained executable guest");

    std::fs::remove_file(&main).unwrap();
    std::fs::write(&main, "pub fnc main(): int { return 42; }\n").unwrap();
    std::fs::remove_file(&manifest).unwrap();
    assert!(Command::new("mkfifo").arg(&manifest).status().unwrap().success());
    let denied_manifest = Command::new("timeout")
        .args(["4s", env!("CARGO_BIN_EXE_attoc"), "--package-root", root,
               "--entry", "main.atto", "-o", output.to_str().unwrap()])
        .output().unwrap();
    assert_eq!(denied_manifest.status.code(), Some(1),
        "FIFO manifest must fail before blocking: {}",
        String::from_utf8_lossy(&denied_manifest.stderr));
    assert!(denied_manifest.stdout.is_empty());
    assert!(String::from_utf8_lossy(&denied_manifest.stderr).contains("E_MANIFEST"));
    assert!(!output.exists(), "failed manifest read retained executable guest");
}
