//! Compiler CLI modes must not launder unverified conditional CFG facts.
use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "oresc-conditional-cli-{}-{nonce}", std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        Self(dir)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_oresc")
}

#[test]
fn checker_accepts_typed_ternary_but_refuses_unmodeled_effect_exports() {
    let dir = Temp::new();
    let guest = dir.0.join("guest.atto");
    fs::write(&guest, include_str!("fixtures/conditional_v1/literal_true.atto"))
        .expect("write guest source");
    let check = Command::new(cli())
        .arg("--check")
        .arg(&guest)
        .output()
        .expect("invoke checker");
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    for option in ["--effects", "--safety-ir", "--cfg-ir", "--static-facts"] {
        let result = Command::new(cli())
            .arg(option)
            .arg(&guest)
            .output()
            .expect("invoke unsupported analyzer");
        assert!(!result.status.success(), "{option} unexpectedly certified");
        let err = String::from_utf8_lossy(&result.stderr);
        assert!(err.contains("branch effects are not modeled"), "{option}: {err}");
    }
    let flow = Command::new(cli())
        .args(["--flow", "32"])
        .arg(&guest)
        .output()
        .expect("invoke unsupported flow exporter");
    assert!(!flow.status.success());
    let object = dir.0.join("forbidden.o");
    let facts = dir.0.join("forbidden.json");
    let paired = Command::new(cli())
        .arg("--compile-with-effects")
        .arg(&guest)
        .arg(&object)
        .arg(&facts)
        .output()
        .expect("invoke paired compiler");
    assert!(!paired.status.success());
    assert!(!object.exists() && !facts.exists(), "unverified artifacts published");
}

#[test]
#[cfg(target_os = "linux")]
fn release_native_retains_callees_in_unselected_ternary_branches() {
    let dir = Temp::new();
    let guest = dir.0.join("release.atto");
    let object = dir.0.join("release.o");
    let binary = dir.0.join("release");
    fs::write(
        &guest,
        "pub fnc main(): int { return true ? chosen() : other(); }\nfnc chosen(): int { return 42; }\nfnc other(): int { return 7; }",
    )
    .expect("write guest");
    let compiled = Command::new(cli())
        .arg("--release")
        .arg(&guest)
        .arg(&object)
        .output()
        .expect("compile release guest");
    assert!(
        compiled.status.success(),
        "release compiler failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let linked = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("link release guest");
    assert!(
        linked.status.success(),
        "release link failed: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let result = Command::new(&binary).status().expect("run release guest");
    assert_eq!(result.code(), Some(42));
}
