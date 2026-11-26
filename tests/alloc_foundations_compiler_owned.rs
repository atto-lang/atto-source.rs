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

#[test]
fn compiled_alignment_and_page_rounding_execute_for_all_admitted_powers() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut source = String::from("import module aligned from \"atto.alloc/aligned\"; pub fnc main(): int {");
    for n in 0..=16 {
        let page = 1i64 << n;
        for bytes in [0, 1, page - 1, page, page + 1, 2147483647] {
            let expected = if bytes == 0 { 0 } else { (bytes - 1) / page + 1 };
            source.push_str(&format!("if aligned.pages_for({bytes},{page}) != {expected} then return 1; fi "));
        }
        let address = 2147483647i64;
        let pad = (page - address % page) % page;
        source.push_str(&format!("if aligned.padding_for({address},{page}) != {pad} then return 2; fi "));
    }
    source.push_str("if aligned.pages_for(42,3) != -1 then return 3; fi if aligned.pages_for(-1,8) != -1 then return 4; fi if aligned.align_up(2147483647,8) != -1 then return 5; fi return 42; }");
    let wasm = compile(&source).expect("compile full bounded alignment matrix");
    let mut child = Command::new("node").arg("-e").arg("const fs=require('node:fs');const m=new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(0)),{});if(m.exports.main()!==42n)process.exitCode=1;")
        .stdin(Stdio::piped()).spawn().expect("Node required for guest execution");
    child.stdin.take().unwrap().write_all(&wasm).unwrap();
    assert!(child.wait().unwrap().success(), "compiled alignment guest failed");
}
