//! Compiler-owned alloc import tests: compiled Wasm evidence only, no claimed
//! native execution or secure memory guarantee.
use atto_compiler::linker::{compile_linked, LinkUnit};

const ENTRY: &str = r#"
import module aligned from "atto.alloc/aligned";
pub fnc main(): int {
    return aligned.align_up(13, 8) + 26;
}
"#;
fn compile(source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_linked("src/main.atto", &[LinkUnit {
        path: "src/main.atto", package: "consumer@1.0.0", source,
    }])
}
#[test]
fn builtin_alignment_is_a_compiled_owned_wasm_module() {
    let wasm = compile(ENTRY).expect("bundled alloc/aligned must compile");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(ENTRY).expect("deterministic compiler output"));
    for alias in ["atto.alloc/aligned.atto", "alloc/aligned", "alloc/aligned.atto"] {
        let candidate = ENTRY.replace("atto.alloc/aligned", alias);
        assert!(compile(&candidate).is_ok(), "canonical alias must resolve: {alias}");
    }
}
#[test]
fn unsupported_allocation_apis_fail_before_emitting_wasm() {
    for path in ["atto.alloc/stats", "atto.alloc/secure", "alloc/stats.atto",
                 "alloc/secure.atto"] {
        let candidate = ENTRY.replace("atto.alloc/aligned", path);
        let diagnostic = compile(&candidate).expect_err("unsupported owned-class API");
        assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{path}");
    }
}
#[test]
fn forged_and_unknown_builtins_are_never_authority() {
    for path in ["atto.alloc/unknown", "atto.alloc/../aligned",
                 "atto.alloc//aligned", "alloc/aligned.ores",
                 "+__atto_builtin/alloc/aligned.atto"] {
        let candidate = ENTRY.replace("atto.alloc/aligned", path);
        let diagnostic = compile(&candidate).expect_err("bad import path");
        assert_eq!(diagnostic.code, "E_IMPORT_PATH", "{path}");
    }
    let shadowed = compile_linked("src/main.atto", &[
        LinkUnit { path: "src/main.atto", package: "consumer@1.0.0", source: ENTRY },
        LinkUnit { path: "__atto_builtin/alloc/aligned.atto",
                   package: "consumer@1.0.0",
                   source: "define pub module aligned as pub fnc align_up(int a,int b):int {return 999;} end" },
    ]).expect_err("user source must not replace compiler library");
    assert_eq!(shadowed.code, "E_IMPORT_PATH");
}

#[test]
fn filesystem_package_resolver_injects_builtin_without_disk_shadow() {
    use atto_compiler::package_resolver::{compile_package, compile_discovered};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "atto-alloc-package-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("atto.alloc")).unwrap();
    fs::write(root.join(".atto.pkg.toml"),
        "[package]\nname=\"consumer\"\nversion=\"1.0.0\"\n").unwrap();
    fs::write(root.join("src/main.atto"), ENTRY).unwrap();
    // A filesystem lookalike may exist but must never become compiler source.
    fs::write(root.join("atto.alloc/aligned.atto"),
        "define pub module aligned as pub fnc align_up(int a,int b):int {return 999;} end"
    ).unwrap();
    let pkg = compile_package(&root, "src/main.atto")
        .expect("real package resolver must link embedded allocator library");
    let discovered = compile_discovered(&root.join("src/main.atto"))
        .expect("discovery must preserve compiler-owned import");
    assert_eq!(pkg, discovered);
    assert_eq!(pkg, compile(ENTRY).expect("same canonical source graph"));
    fs::remove_dir_all(&root).unwrap();
}
