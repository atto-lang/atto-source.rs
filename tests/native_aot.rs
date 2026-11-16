#![cfg(feature = "native-aot")]
//! Rust-only native host integration checks. Actual native AOT materialization
//! requires Wasmtime; when ATTO_REQUIRE_NATIVE_AOT=1, absence is a hard FAIL.
//! A green compiler-unit test by itself is not native machine-code proof.
use std::{env,ffi::OsString,fs,path::{Path,PathBuf},process::Command};

struct Work(PathBuf);
impl Work {
    fn new(label: &str) -> Self {
        for n in 0..100 {
            let p = env::temp_dir().join(format!("atto-aot-test-{}-{label}-{n}", std::process::id()));
            match fs::create_dir(&p) {
                Ok(()) => return Work(p),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create AOT test workdir: {e}"),
            }
        }
        panic!("too many AOT test workdirs");
    }
    fn path(&self, filename: &str) -> PathBuf { self.0.join(filename) }
}
impl Drop for Work {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn attoaot() -> &'static str { env!("CARGO_BIN_EXE_attoaot") }
fn compile(source: &Path, output: &Path, wasmtime: &OsString) -> std::process::Output {
    Command::new(attoaot()).arg(source).arg("-o").arg(output)
        .arg("--wasmtime").arg(wasmtime).output().expect("start attoaot")
}

#[test]
fn aot_is_fail_closed_when_external_native_compiler_is_missing() {
    let w = Work::new("missing");
    let source = w.path("source.atto");
    let output = w.path("output.cwasm");
    fs::write(&source, "pub fnc main(): int { return 42; }").unwrap();
    let result = compile(&source, &output, &OsString::from(w.path("definitely-not-installed")));
    assert!(!result.status.success());
    assert!(!output.exists(), "a failure must not publish a fake AOT artifact");
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot start Wasmtime"));
    assert!(!w.path("source.wasm").exists(), "no intermediate Wasm is published");
}

#[test]
fn aot_rejects_host_imports_until_native_runtime_host_is_implemented() {
    let w = Work::new("imports");
    let source = w.path("imports.atto");
    let output = w.path("imports.cwasm");
    fs::write(&source, "pub fnc main(): int { stdio.println(\"hello\"); return 0; }").unwrap();
    let result = compile(&source, &output, &OsString::from(w.path("no-wasmtime")));
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("E_AOT_HOST_IMPORTS"),
        "expected precise rejection, got {}", String::from_utf8_lossy(&result.stderr));
    assert!(!output.exists());
}

#[test]
fn aot_machine_code_is_built_ahead_of_execution_and_runs_without_source() {
    let wasmtime = env::var_os("ATTO_WASMTIME").unwrap_or_else(|| OsString::from("wasmtime"));
    let available = Command::new(&wasmtime).arg("--version").output()
        .is_ok_and(|x| x.status.success());
    if !available {
        if env::var("ATTO_REQUIRE_NATIVE_AOT").as_deref() == Ok("1") {
            panic!("AOT admission requires an installed Wasmtime binary; fail closed");
        }
        eprintln!("NOT_EXECUTED: Wasmtime native AOT runtime is not installed");
        return;
    }
    let w = Work::new("actual");
    let source = w.path("source.atto");
    let output = w.path("module.cwasm");
    // Reuse the actual new zero-copy sliced Array<int> profile from PR #178.
    fs::write(&source, "pub fnc main(): int { val a=[10,20,30,40,50]; return a[1:4][1]; }").unwrap();
    let result = compile(&source, &output, &wasmtime);
    assert!(result.status.success(), "native AOT compiler failed:\n{}",
        String::from_utf8_lossy(&result.stderr));
    assert!(fs::metadata(&output).unwrap().len() > 0);
    assert_ne!(&fs::read(&output).unwrap()[..4], b"\0asm",
        "output must be precompiled native Wasmtime module, not uncompiled Wasm");
    fs::remove_file(&source).unwrap();
    // Only native .cwasm is left; the source and intermediate Wasm are absent.
    assert!(!source.exists());
    assert!(!w.path("module.wasm").exists());
    let run = Command::new(&wasmtime).arg("run").arg("--invoke").arg("main")
        .arg(&output).output().expect("start native AOT runtime");
    assert!(run.status.success(), "precompiled native module failed:\n{}",
        String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.lines().any(|line| line.trim() == "30"),
        "AttoLang guest produced unexpected native AOT result: {stdout}");
    eprintln!("NATIVE_AOT_EXECUTED: exact precompiled module produced 30 from Array<int> slice");
}

#[test]
fn aot_never_overwrites_existing_outputs() {
    let w = Work::new("no-overwrite");
    let source = w.path("source.atto");
    let output = w.path("existing.cwasm");
    fs::write(&source, "pub fnc main(): int { return 1; }").unwrap();
    fs::write(&output, "trusted previous artifact").unwrap();
    let result = compile(&source, &output, &OsString::from(w.path("no-wasmtime")));
    assert!(!result.status.success());
    assert_eq!(fs::read_to_string(&output).unwrap(), "trusted previous artifact");
}
