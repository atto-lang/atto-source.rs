//! Host-side package path and manifest security regression tests.
use atto_compiler::package_resolver::{
    compile_discovered, compile_package, compile_package_locked, discover_package_root, parse_package_identity,
};
use atto_compiler::sha256::hex_digest;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const MAIN: &str = r#"use namespace shop;
import module Orders from "+lib/orders.atto";
pub fnc main(): int { return Orders.total(21); }"#;
const ORDERS: &str = r#"use namespace sales;
import module Math from "./math.atto";
export pub module Orders;
define module Orders as
 pub fnc total(int x): int { return Math.double(x); }
end"#;
const MATH: &str = r#"use namespace math;
export pub[pkg] module Math;
define module Math as
 pub fnc double(int x): int { return x * 2; }
end"#;

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("atto-pkg-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let f = Self { root };
        f.write(".atto.pkg.toml", "[package]\nname = \"storefront\"\nversion = \"1.2.3\"\n");
        f
    }
    fn write(&self, rel: &str, data: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, data).unwrap();
    }
    fn root(&self) -> &Path {
        &self.root
    }
    fn with_graph(&self) {
        self.write("src/main.atto", MAIN);
        self.write("lib/orders.atto", ORDERS);
        self.write("lib/math.atto", MATH);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn rejects(root: &Path, entry: &str, code: &str) {
    let e = compile_package(root, entry).expect_err("invalid file must not produce guest");
    assert_eq!(e.code, code, "unexpected diagnostic {e}");
}

#[test]
fn strict_manifest_identity_is_stable_and_rejects_forgery() {
    let valid = "[package]\nname=\"storefront\"\nversion=\"1.2.3-rc1\"\n";
    assert_eq!(parse_package_identity(valid).unwrap(), "storefront@1.2.3-rc1");
    for bad in [
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\nname=\"b\"\n",
        "[package]\nname=\"a/b\"\nversion=\"1.2.3\"\n",
        "[package]\nname=\"a\"\nversion=\"1.2\"\n",
        "[other]\nname=\"a\"\nversion=\"1.2.3\"\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\n[dependencies]\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\n[package]\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"garbage\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\npublish=true\n",
    ] {
        assert_eq!(parse_package_identity(bad).unwrap_err().code, "E_MANIFEST", "{bad}");
    }
}

#[test]
fn manifest_root_and_relative_imports_produce_real_linked_wasm() {
    let f = Fixture::new();
    f.with_graph();
    let a = compile_package(f.root(), "src/main.atto").expect("trusted single-package graph");
    let b = compile_package(f.root(), "src/main.atto").expect("deterministic repeat");
    assert_eq!(a, b);
    assert!(a.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn manifest_and_package_path_cannot_be_traversed() {
    let f = Fixture::new();
    f.with_graph();
    for entry in ["../main.atto", "src/../main.atto", "/etc/passwd", "src//main.atto"] {
        rejects(f.root(), entry, "E_IMPORT_PATH");
    }
    f.write("src/main.atto", &MAIN.replace("+lib/orders.atto", "+../lib/orders.atto"));
    rejects(f.root(), "src/main.atto", "E_IMPORT_PATH");
}

#[test]
fn a_different_directory_cannot_become_the_same_package_by_name() {
    let a = Fixture::new();
    let b = Fixture::new();
    b.write("lib/orders.atto", ORDERS);
    b.write("lib/math.atto", MATH);
    a.write("src/main.atto", MAIN);
    // Both package manifests have the same name/version, but physical roots
    // are independent. Sources cannot be loaded from the sibling root.
    rejects(a.root(), "src/main.atto", "E_IMPORT_PATH");
}

#[test]
fn private_module_members_and_malformed_manifests_still_fail_on_disk() {
    let f = Fixture::new();
    f.with_graph();
    f.write("lib/orders.atto", &ORDERS.replace("Math.double(x)", "Math.secret()"));
    rejects(f.root(), "src/main.atto", "E_UNBOUND");
    f.write(
        ".atto.pkg.toml",
        "[package]\nname=\"storefront\"\nname=\"different\"\nversion=\"1.2.3\"\n",
    );
    rejects(f.root(), "src/main.atto", "E_MANIFEST");
}

#[cfg(unix)]
#[test]
fn symlinks_to_files_and_directories_are_rejected_before_loading() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.with_graph();
    let alt = f.root.join("lib/other.atto");
    fs::write(&alt, MATH).unwrap();
    fs::remove_file(f.root.join("lib/math.atto")).unwrap();
    symlink(&alt, f.root.join("lib/math.atto")).unwrap();
    rejects(f.root(), "src/main.atto", "E_IMPORT_PATH");
    fs::remove_file(f.root.join("lib/math.atto")).unwrap();
    fs::write(f.root.join("lib/math.atto"), MATH).unwrap();
    fs::rename(f.root.join("lib"), f.root.join("safe_lib")).unwrap();
    symlink(f.root.join("safe_lib"), f.root.join("lib")).unwrap();
    rejects(f.root(), "src/main.atto", "E_IMPORT_PATH");
}

#[cfg(unix)]
#[test]
fn symlink_package_root_and_manifest_are_not_trusted() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.with_graph();
    let root_alias = f.root.with_extension("alias");
    symlink(f.root(), &root_alias).unwrap();
    rejects(&root_alias, "src/main.atto", "E_IMPORT_PATH");
    fs::remove_file(&root_alias).unwrap();
    fs::rename(f.root.join(".atto.pkg.toml"), f.root.join("alternate.toml")).unwrap();
    symlink(f.root.join("alternate.toml"), f.root.join(".atto.pkg.toml")).unwrap();
    rejects(f.root(), "src/main.atto", "E_MANIFEST");
}

#[test]
fn nearest_manifest_discovery_returns_verified_root_and_source_relative_path() {
    let f = Fixture::new();
    f.with_graph();
    let entry = f.root().join("src/main.atto");
    let (root, relative) = discover_package_root(&entry).expect("find root");
    assert_eq!(root, fs::canonicalize(f.root()).unwrap());
    assert_eq!(relative, "src/main.atto");
    let a = compile_discovered(&entry).expect("compiled discovered package");
    let b = compile_package(f.root(), "src/main.atto").expect("explicit root");
    assert_eq!(a, b, "discovery must not alter compiler semantics");
}
#[test]
fn discovery_rejects_missing_manifest_and_parent_directory_input() {
    let f = Fixture::new();
    f.write("src/main.atto", "pub fnc main(): int { return 42; }");
    fs::remove_file(f.root().join(".atto.pkg.toml")).unwrap();
    assert_eq!(
        discover_package_root(&f.root().join("src/main.atto")).unwrap_err().code,
        "E_MANIFEST"
    );
    assert_eq!(
        discover_package_root(&f.root().join("src/../src/main.atto")).unwrap_err().code,
        "E_IMPORT_PATH"
    );
}
#[cfg(unix)]
#[test]
fn discovery_rejects_symlink_source_alias_before_package_scan() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.with_graph();
    let alias = f.root().join("src/alias.atto");
    symlink(f.root().join("src/main.atto"), &alias).unwrap();
    assert_eq!(discover_package_root(&alias).unwrap_err().code, "E_IMPORT_PATH");
}

fn trusted_lock(f: &Fixture) -> String {
    let manifest = fs::read_to_string(f.root().join(".atto.pkg.toml")).unwrap();
    let mut lock = format!("[lock]\nmanifest_sha256=\"{}\"\n[files]\n", hex_digest(manifest.as_bytes()));
    for path in ["lib/math.atto", "lib/orders.atto", "src/main.atto"] {
        let bytes = fs::read(f.root().join(path)).unwrap();
        lock.push_str(&format!("\"{path}\"=\"{}\"\n", hex_digest(&bytes)));
    }
    f.write(".atto.pkg.lock", &lock);
    hex_digest(lock.as_bytes())
}
#[test]
fn authenticated_package_lock_pins_manifest_and_transitive_sources() {
    let f = Fixture::new();
    f.with_graph();
    let expected = trusted_lock(&f);
    let a = compile_package_locked(f.root(), "src/main.atto", &expected).expect("valid trusted package lock");
    assert_eq!(a, compile_package(f.root(), "src/main.atto").unwrap());
    // Change transitive content without changing the manifest's public name.
    f.write("lib/math.atto", &MATH.replace("x * 2", "x * 3"));
    assert_eq!(
        compile_package_locked(f.root(), "src/main.atto", &expected).unwrap_err().code,
        "E_LOCK_HASH"
    );
    f.write("lib/math.atto", MATH);
    f.write(".atto.pkg.toml", "[package]\nname=\"imposter\"\nversion=\"1.2.3\"\n");
    assert_eq!(
        compile_package_locked(f.root(), "src/main.atto", &expected).unwrap_err().code,
        "E_LOCK_HASH"
    );
}
#[test]
fn caller_trusted_lock_digest_prevents_rewriting_both_lock_and_sources() {
    let f = Fixture::new();
    f.with_graph();
    let expected = trusted_lock(&f);
    let bad = fs::read_to_string(f.root().join(".atto.pkg.lock")).unwrap();
    f.write(".atto.pkg.lock", &bad.replace("manifest_sha256", "manifext_sha256"));
    assert_eq!(
        compile_package_locked(f.root(), "src/main.atto", &expected).unwrap_err().code,
        "E_LOCK_HASH"
    );
    assert_eq!(
        compile_package_locked(f.root(), "src/main.atto", &"a".repeat(63)).unwrap_err().code,
        "E_LOCK"
    );
}
#[test]
fn locked_build_rejects_stale_and_extra_source_records() {
    let f = Fixture::new();
    f.with_graph();
    let expected = trusted_lock(&f);
    let old = fs::read_to_string(f.root().join(".atto.pkg.lock")).unwrap();
    let altered = old.replace("[files]", "[files]\n\"not_needed.atto\"=\"".to_string().as_str());
    // A syntactically invalid lock must fail with E_LOCK if signed as such.
    f.write(".atto.pkg.lock", &altered);
    let new_digest = hex_digest(altered.as_bytes());
    assert_eq!(
        compile_package_locked(f.root(), "src/main.atto", &new_digest).unwrap_err().code,
        "E_LOCK"
    );
    f.write(
        ".atto.pkg.lock",
        &old.replace(
            "[files]",
            "[files]\n\"extra.atto\"=\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\n",
        ),
    );
    let contents = fs::read(f.root().join(".atto.pkg.lock")).unwrap();
    assert_eq!(
        compile_package_locked(f.root(), "src/main.atto", &hex_digest(&contents))
            .unwrap_err()
            .code,
        "E_LOCK_HASH"
    );
    f.write(".atto.pkg.lock", &old);
    assert!(compile_package_locked(f.root(), "src/main.atto", &expected).is_ok());
}

#[test]
fn authenticated_immutable_snapshot_compiles_without_filesystem() {
    use atto_compiler::package_resolver::{compile_locked_snapshot, SnapshotSource};
    let manifest = "[package]\nname=\"shop\"\nversion=\"1.0.0\"\n";
    let mut lock = format!("[lock]\nmanifest_sha256=\"{}\"\n[files]\n", hex_digest(manifest.as_bytes()));
    for (path, source) in [("lib/math.atto", MATH), ("lib/orders.atto", ORDERS), ("src/main.atto", MAIN)] {
        lock.push_str(&format!("\"{path}\"=\"{}\"\n", hex_digest(source.as_bytes())));
    }
    let digest = hex_digest(lock.as_bytes());
    let snapshot = [
        SnapshotSource {
            path: "src/main.atto",
            source: MAIN,
        },
        SnapshotSource {
            path: "lib/math.atto",
            source: MATH,
        },
        SnapshotSource {
            path: "lib/orders.atto",
            source: ORDERS,
        },
    ];
    let guest = compile_locked_snapshot("src/main.atto", manifest, &lock, &digest, &snapshot).expect("valid immutable snapshot");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    let reordered = [snapshot[2], snapshot[1], snapshot[0]];
    assert_eq!(
        guest,
        compile_locked_snapshot("src/main.atto", manifest, &lock, &digest, &reordered).unwrap()
    );
    let modified = MATH.replace("x * 2", "x * 3");
    let forged = [
        snapshot[0],
        SnapshotSource {
            path: "lib/math.atto",
            source: &modified,
        },
        snapshot[2],
    ];
    assert_eq!(
        compile_locked_snapshot("src/main.atto", manifest, &lock, &digest, &forged)
            .unwrap_err()
            .code,
        "E_LOCK_HASH"
    );
    assert_eq!(
        compile_locked_snapshot("src/main.atto", manifest, &lock, &"0".repeat(64), &snapshot)
            .unwrap_err()
            .code,
        "E_LOCK_HASH"
    );
    assert_eq!(
        compile_locked_snapshot("src/main.atto", manifest, &lock, &digest, &snapshot[..2])
            .unwrap_err()
            .code,
        "E_LOCK_HASH"
    );
    let dupe = [snapshot[0], snapshot[0], snapshot[2]];
    assert_eq!(
        compile_locked_snapshot("src/main.atto", manifest, &lock, &digest, &dupe)
            .unwrap_err()
            .code,
        "E_DUPLICATE"
    );
}

#[test]
fn signed_snapshot_rejects_unreachable_extra_content() {
    use atto_compiler::package_resolver::{compile_locked_snapshot, SnapshotSource};
    let manifest = "[package]\nname=\"shop\"\nversion=\"1.0.0\"\n";
    let other = "define module Unused as end";
    let mut lock = format!("[lock]\nmanifest_sha256=\"{}\"\n[files]\n", hex_digest(manifest.as_bytes()));
    for (path, source) in [
        ("lib/math.atto", MATH),
        ("lib/orders.atto", ORDERS),
        ("src/main.atto", MAIN),
        ("lib/unused.atto", other),
    ] {
        lock.push_str(&format!("\"{path}\"=\"{}\"\n", hex_digest(source.as_bytes())));
    }
    let sources = [
        SnapshotSource {
            path: "lib/math.atto",
            source: MATH,
        },
        SnapshotSource {
            path: "lib/orders.atto",
            source: ORDERS,
        },
        SnapshotSource {
            path: "src/main.atto",
            source: MAIN,
        },
        SnapshotSource {
            path: "lib/unused.atto",
            source: other,
        },
    ];
    assert_eq!(
        compile_locked_snapshot("src/main.atto", manifest, &lock, &hex_digest(lock.as_bytes()), &sources)
            .unwrap_err()
            .code,
        "E_LOCK_HASH"
    );
}
