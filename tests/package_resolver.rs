//! Host-side package path and manifest security regression tests.
use atto_compiler::package_resolver::{compile_package, parse_package_identity};
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

struct Fixture { root: PathBuf }
impl Fixture {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("atto-pkg-{}-{id}",std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let f = Self { root };
        f.write(".atto.pkg.toml","[package]\nname = \"storefront\"\nversion = \"1.2.3\"\n");
        f
    }
    fn write(&self, rel: &str, data: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).unwrap(); }
        fs::write(path, data).unwrap();
    }
    fn root(&self) -> &Path { &self.root }
    fn with_graph(&self) {
        self.write("src/main.atto", MAIN);
        self.write("lib/orders.atto", ORDERS);
        self.write("lib/math.atto", MATH);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}
fn rejects(root: &Path, entry: &str, code: &str) {
    let e = compile_package(root, entry).expect_err("invalid file must not produce guest");
    assert_eq!(e.code,code,"unexpected diagnostic {e}");
}

#[test]
fn strict_manifest_identity_is_stable_and_rejects_forgery() {
    let valid="[package]\nname=\"storefront\"\nversion=\"1.2.3-rc1\"\n";
    assert_eq!(parse_package_identity(valid).unwrap(),"storefront@1.2.3-rc1");
    for bad in [
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\nname=\"b\"\n",
        "[package]\nname=\"a/b\"\nversion=\"1.2.3\"\n",
        "[package]\nname=\"a\"\nversion=\"1.2\"\n",
        "[other]\nname=\"a\"\nversion=\"1.2.3\"\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\n[dependencies]\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\n[package]\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"garbage\n",
        "[package]\nname=\"a\"\nversion=\"1.2.3\"\npublish=true\n",
    ] { assert_eq!(parse_package_identity(bad).unwrap_err().code, "E_MANIFEST", "{bad}"); }
}

#[test]
fn manifest_root_and_relative_imports_produce_real_linked_wasm() {
    let f=Fixture::new();
    f.with_graph();
    let a=compile_package(f.root(),"src/main.atto").expect("trusted single-package graph");
    let b=compile_package(f.root(),"src/main.atto").expect("deterministic repeat");
    assert_eq!(a,b);
    assert!(a.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn manifest_and_package_path_cannot_be_traversed() {
    let f=Fixture::new();
    f.with_graph();
    for entry in ["../main.atto","src/../main.atto","/etc/passwd","src//main.atto"] {
        rejects(f.root(),entry,"E_IMPORT_PATH");
    }
    f.write("src/main.atto",&MAIN.replace("+lib/orders.atto","+../lib/orders.atto"));
    rejects(f.root(),"src/main.atto","E_IMPORT_PATH");
}

#[test]
fn a_different_directory_cannot_become_the_same_package_by_name() {
    let a=Fixture::new();
    let b=Fixture::new();
    b.write("lib/orders.atto",ORDERS);
    b.write("lib/math.atto",MATH);
    a.write("src/main.atto",MAIN);
    // Both package manifests have the same name/version, but physical roots
    // are independent. Sources cannot be loaded from the sibling root.
    rejects(a.root(),"src/main.atto","E_IMPORT_PATH");
}

#[test]
fn private_module_members_and_malformed_manifests_still_fail_on_disk() {
    let f=Fixture::new();
    f.with_graph();
    f.write("lib/orders.atto",&ORDERS.replace("Math.double(x)","Math.secret()"));
    rejects(f.root(),"src/main.atto","E_UNBOUND");
    f.write(".atto.pkg.toml","[package]\nname=\"storefront\"\nname=\"different\"\nversion=\"1.2.3\"\n");
    rejects(f.root(),"src/main.atto","E_MANIFEST");
}

#[cfg(unix)]
#[test]
fn symlinks_to_files_and_directories_are_rejected_before_loading() {
    use std::os::unix::fs::symlink;
    let f=Fixture::new();
    f.with_graph();
    let alt=f.root.join("lib/other.atto");
    fs::write(&alt,MATH).unwrap();
    fs::remove_file(f.root.join("lib/math.atto")).unwrap();
    symlink(&alt,f.root.join("lib/math.atto")).unwrap();
    rejects(f.root(),"src/main.atto","E_IMPORT_PATH");
    fs::remove_file(f.root.join("lib/math.atto")).unwrap();
    fs::write(f.root.join("lib/math.atto"),MATH).unwrap();
    fs::rename(f.root.join("lib"),f.root.join("safe_lib")).unwrap();
    symlink(f.root.join("safe_lib"),f.root.join("lib")).unwrap();
    rejects(f.root(),"src/main.atto","E_IMPORT_PATH");
}

#[cfg(unix)]
#[test]
fn symlink_package_root_and_manifest_are_not_trusted() {
    use std::os::unix::fs::symlink;
    let f=Fixture::new();
    f.with_graph();
    let root_alias=f.root.with_extension("alias");
    symlink(f.root(),&root_alias).unwrap();
    rejects(&root_alias,"src/main.atto","E_IMPORT_PATH");
    fs::remove_file(&root_alias).unwrap();
    fs::rename(f.root.join(".atto.pkg.toml"),f.root.join("alternate.toml")).unwrap();
    symlink(f.root.join("alternate.toml"),f.root.join(".atto.pkg.toml")).unwrap();
    rejects(f.root(),"src/main.atto","E_MANIFEST");
}
