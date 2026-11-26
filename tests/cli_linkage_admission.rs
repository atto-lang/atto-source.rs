use std::{fs, path::PathBuf, process::Command, sync::atomic::{AtomicU64, Ordering}};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("atto-linkage-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&p).unwrap();
        fs::write(p.join("main.atto"), "pub fnc main(): int { return 42; }\n").unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_attoc")).current_dir(&self.0).args(args).output().unwrap()
    }
}
impl Drop for Scratch { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

#[test]
fn explicit_static_matches_existing_single_linked_guest() {
    let s = Scratch::new();
    let a = s.run(&["main.atto", "-o", "default.wasm"]);
    assert!(a.status.success(), "{:?}", a);
    let b = s.run(&["--link=static", "main.atto", "-o", "static.wasm"]);
    assert!(b.status.success(), "{:?}", b);
    let bytes = fs::read(s.0.join("default.wasm")).unwrap();
    assert!(bytes.starts_with(b"\0asm"));
    assert_eq!(bytes, fs::read(s.0.join("static.wasm")).unwrap());
}

#[test]
fn unsupported_modes_fail_before_source_or_output_access() {
    let s = Scratch::new();
    for mode in ["--link=dynamic", "--link=hybrid"] {
        let a = s.run(&[mode, "--package-root", ".", "--entry", "missing.atto", "-o", "new.wasm"]);
        assert_eq!(a.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&a.stderr).contains("E_LINK_MODE_UNSUPPORTED"));
        assert!(!s.0.join("new.wasm").exists());
        fs::write(s.0.join("old.wasm"), b"prior").unwrap();
        let b = s.run(&["main.atto", "-o", "old.wasm", mode]);
        assert_eq!(b.status.code(), Some(2));
        assert_eq!(fs::read(s.0.join("old.wasm")).unwrap(), b"prior");
    }
}

#[test]
fn invalid_or_duplicate_modes_fail_closed() {
    let s = Scratch::new();
    for args in [vec!["--link=unknown"], vec!["--link=static", "--link=static"]] {
        let mut a = args;
        a.extend(["main.atto", "-o", "new.wasm"]);
        let out = s.run(&a);
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("E_LINK_MODE"));
        assert!(!s.0.join("new.wasm").exists());
    }
}
