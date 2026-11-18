//! External profile selection: the native compiler must never guess whether
//! an unversioned callee's class formal consumes the owner's storage.
use std::{fs, path::PathBuf, process::{Command, Output}};

fn source_file(name: &str, content: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "atto_owner_profile_{}_{}_{}.atto", std::process::id(), name,
        env!("CARGO_PKG_VERSION").replace('.', "_")
    ));
    fs::write(&path, content).expect("create source fixture");
    path
}

fn run(path: &PathBuf, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_attoc"));
    cmd.arg(path).args(args);
    cmd.output().expect("run real native attoc binary")
}

#[test]
fn explicit_driver_profile_selects_borrowing_without_injecting_a_comment() {
    let source = r#"
        define class Parcel as pub let int value = 42; end
        fnc inspect(Parcel p): int { return p.value; }
        pub fnc main(): int {
            let parcel = new Parcel();
            val result = inspect(parcel);
            return result + parcel.value;
        }
    "#;
    let path = source_file("external", source);
    let legacy = run(&path, &[]);
    assert!(!legacy.status.success(), "legacy consuming Foo must still reject owner reuse");
    assert_eq!(legacy.stdout.len(), 0, "no rejected source guest artifact");
    assert!(String::from_utf8_lossy(&legacy.stderr).contains("E_MOVED"));

    for flag in [
        vec!["--ownership-profile", "borrow-default-v1"],
        vec!["--ownership-profile=borrow-default-v1"],
    ] {
        let external = run(&path, &flag);
        assert!(external.status.success(), "external profile failure: {}",
            String::from_utf8_lossy(&external.stderr));
        assert!(external.stdout.starts_with(&[0, 97, 115, 109]));
    }
    fs::remove_file(path).ok();
}

#[test]
fn source_marker_conflict_and_unknown_profiles_reject_without_guest_bytes() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub let int value = 42; end
        fnc inspect(Parcel p): int { return p.value; }
        pub fnc main(): int {
            let parcel = new Parcel();
            return inspect(parcel) + parcel.value;
        }
    "#;
    let path = source_file("conflict", source);
    let conflict = run(&path, &["--ownership-profile", "legacy"]);
    assert!(!conflict.status.success());
    assert_eq!(conflict.stdout.len(), 0);
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("E_PROFILE"));
    let match_profile = run(&path, &["--ownership-profile", "borrow-default-v1"]);
    assert!(match_profile.status.success(), "{:?}", match_profile.stderr);
    assert!(match_profile.stdout.starts_with(&[0, 97, 115, 109]));

    let unknown = run(&path, &["--ownership-profile", "experimental-v99"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(unknown.stdout.is_empty());
    let duplicate = run(&path, &["--ownership-profile", "legacy", "--ownership-profile", "legacy"]);
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(duplicate.stdout.is_empty());
    fs::remove_file(path).ok();
}

#[test]
fn output_file_is_not_created_on_ownership_error() {
    let source = r#"
        define class Parcel as pub let int value = 42; end
        fnc inspect(Parcel p): int { return p.value; }
        pub fnc main(): int {
            let parcel = new Parcel();
            return inspect(parcel) + parcel.value;
        }
    "#;
    let source_path = source_file("negative_artifact", source);
    let output = source_path.with_extension("wasm");
    fs::remove_file(&output).ok();
    let result = run(&source_path, &["-o", output.to_str().expect("utf8 test path"), "--ownership-profile", "legacy"]);
    assert!(!result.status.success());
    assert!(!output.exists(), "invalid ownership compiled a guest output file");
    fs::remove_file(source_path).ok();
}
