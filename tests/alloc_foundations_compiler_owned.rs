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
