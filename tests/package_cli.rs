//! Native attoc CLI package mode and backwards-compatibility tests.
use std::fs;
use std::process::Command;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace { root: PathBuf }
impl Workspace {
    fn new() -> Self {
        let i = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("atto-cli-{}-{i}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join(".atto.pkg.toml"),
            "[package]\nname = \"cli_fixture\"\nversion = \"1.0.0\"\n").unwrap();
        fs::write(root.join("src/main.atto"),
            "use namespace main;\nimport module Math from \"+lib/math.atto\";\npub fnc main(): int { return Math.double(21); }"
        ).unwrap();
        fs::write(root.join("lib/math.atto"),
            "export pub[pkg] module Math;\ndefine module Math as\npub fnc double(int x): int { return x * 2; }\nend"
        ).unwrap();
        Self { root }
    }
}
impl Drop for Workspace {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_attoc"))
        .args(args)
        .output()
        .expect("attoc CLI must spawn")
}

#[test]
fn package_mode_creates_real_wasm_instead_of_trusting_caller_identity() {
    let w = Workspace::new();
    let out = w.root.join("out.wasm");
    let result = cli(&["--package-root", w.root.to_str().unwrap(),
        "--entry", "src/main.atto", "-o", out.to_str().unwrap()]);
    assert!(result.status.success(), "{:?}", String::from_utf8_lossy(&result.stderr));
    assert!(fs::read(out).unwrap().starts_with(b"\0asm\x01\0\0\0"));
}
#[test]
fn legacy_single_file_cli_still_compiles() {
    let w=Workspace::new();
    let source=w.root.join("one.atto");
    fs::write(&source,"pub fnc main(): int { return 42; }").unwrap();
    let result=cli(&[source.to_str().unwrap()]);
    assert!(result.status.success(), "{:?}",String::from_utf8_lossy(&result.stderr));
    assert!(result.stdout.starts_with(b"\0asm\x01\0\0\0"));
}
#[test]
fn bad_flag_combinations_fail_before_compilation() {
    let result=cli(&["--package-root","somewhere","--bad","src/main.atto"]);
    assert_eq!(result.status.code(),Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("usage:"));
}
#[test]
fn package_mode_rejects_untrusted_manifest_and_traversal_without_guest() {
    let w=Workspace::new();
    let out=w.root.join("out.wasm");
    let result=cli(&["--package-root",w.root.to_str().unwrap(),
        "--entry","../outside.atto","-o",out.to_str().unwrap()]);
    assert_eq!(result.status.code(),Some(1));
    assert!(!out.exists(),"invalid package must not create output artifact");
    fs::write(w.root.join(".atto.pkg.toml"),
        "[package]\nname=\"cli_fixture\"\nversion=\"1.0.0\"\nversion=\"1.0.1\"").unwrap();
    let result=cli(&["--package-root",w.root.to_str().unwrap(),
        "--entry","src/main.atto","-o",out.to_str().unwrap()]);
    assert_eq!(result.status.code(),Some(1));
    assert!(!out.exists(),"invalid manifest must not create output artifact");
}

#[test]
fn discovered_cli_package_root_compiles_and_matches_explicit_mode() {
    let w=Workspace::new();
    let entry=w.root.join("src/main.atto");
    let explicit=w.root.join("explicit.wasm");
    let automatic=w.root.join("auto.wasm");
    let a=cli(&["--package-root",w.root.to_str().unwrap(),
        "--entry","src/main.atto","-o",explicit.to_str().unwrap()]);
    let b=cli(&["--package",entry.to_str().unwrap(),
        "-o",automatic.to_str().unwrap()]);
    assert!(a.status.success(),"{}",String::from_utf8_lossy(&a.stderr));
    assert!(b.status.success(),"{}",String::from_utf8_lossy(&b.stderr));
    assert_eq!(fs::read(explicit).unwrap(),fs::read(automatic).unwrap());
}
#[test]
fn discovered_cli_never_falls_back_to_untrusted_single_file_mode() {
    let w=Workspace::new();
    fs::remove_file(w.root.join(".atto.pkg.toml")).unwrap();
    let entry=w.root.join("src/main.atto");
    let output=w.root.join("unexpected.wasm");
    let result=cli(&["--package",entry.to_str().unwrap(),
        "-o",output.to_str().unwrap()]);
    assert_eq!(result.status.code(),Some(1));
    assert!(!output.exists());
}
