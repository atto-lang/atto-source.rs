//! First compiled relative-file import profile: actual Wasm, never parser-only.
use atto_compiler::{compile, compile_with_imports};
use std::io;
use std::path::Path;

const MATH: &str = "pub fnc square(int x): int { return x * x; }";
fn build(root: &str, math: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_with_imports(root, Path::new("project/main.atto"), |path| {
        if path == Path::new("project/math.atto") {
            Ok(math.to_string())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }
    })
}
fn code(root: &str, math: &str) -> &'static str {
    let err = build(root, math).expect_err("unsafe or unsupported import must not compile");
    assert!(err.pos.line > 0);
    err.code
}
fn wasm(root: &str, math: &str) -> Vec<u8> {
    let bytes = build(root, math).expect("real production compilation should succeed");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, build(root, math).unwrap(), "deterministic linked bytes");
    bytes
}

#[test]
fn selective_relative_import_emits_real_wasm() {
    wasm("import square from \"./math.atto\"; pub fnc main(): int { return square(7); }", MATH);
}
#[test]
fn alias_and_namespace_imports_emit_real_wasm() {
    wasm("import square as calc from \"./math.atto\"; pub fnc main(): int { return calc(7); }", MATH);
    wasm("import * as math from \"./math.atto\"; pub fnc main(): int { return math.square(7); }", MATH);
}
#[test]
fn private_helper_is_available_inside_imported_module_only() {
    let math = "fnc twice(int x): int { return x + x; } pub fnc square(int x): int { return twice(x) + x * (x - 2); }";
    wasm("import square from \"./math.atto\"; pub fnc main(): int { return square(7); }", math);
    assert_eq!(code("import twice from \"./math.atto\"; pub fnc main(): int { return twice(1); }", math), "E_PRIVATE");
    assert_eq!(code("import * as m from \"./math.atto\"; pub fnc main(): int { return m.twice(1); }", math), "E_PRIVATE");
}
#[test]
fn imported_symbols_must_be_public_and_bound() {
    assert_eq!(code("import not_exported from \"./math.atto\"; pub fnc main(): int {return 0;}", MATH), "E_UNBOUND");
    assert_eq!(code("import square as main from \"./math.atto\"; pub fnc main(): int {return 0;}", MATH), "E_DUPLICATE");
    assert_eq!(code("import square as sq, square as sq from \"./math.atto\"; pub fnc main(): int {return sq(7);}", MATH),
        "E_DUPLICATE");
}
#[test]
fn imports_reject_paths_that_escape_or_do_not_exist() {
    assert_eq!(code("import foo from \"../../secret.atto\"; pub fnc main(): int {return 0;}", MATH), "E_IMPORT_PATH");
    assert_eq!(code("import foo from \"./secret.txt\"; pub fnc main(): int {return 0;}", MATH), "E_IMPORT_PATH");
    assert_eq!(code("import foo from \"./absent.atto\"; pub fnc main(): int {return 0;}", MATH), "E_IMPORT");
}
#[test]
fn unsupported_package_and_transitive_imports_fail_closed() {
    assert_eq!(code("import foo from pkg \"core.telemetry\"; pub fnc main(): int {return 0;}", MATH), "E_UNSUPPORTED");
    let dep = "import other from \"./other.atto\"; pub fnc square(int x): int {return x*x;}";
    assert_eq!(code("import square from \"./math.atto\"; pub fnc main(): int {return square(7);}", dep), "E_UNSUPPORTED");
}
#[test]
fn untrusted_imports_cannot_create_forged_compiler_identifiers() {
    assert_eq!(code("import square from \"./math.atto\"; pub fnc main(): int { return __atto_file_import_0.square(7); }", MATH),
        "E_RESERVED");
    let dep = "fnc __atto_file_import_0(): int {return 0;} pub fnc square(int x): int {return x*x;}";
    assert_eq!(code("import square from \"./math.atto\"; pub fnc main(): int {return square(7);}", dep), "E_RESERVED");
}
#[test]
fn browser_single_file_abi_still_rejects_unresolved_imports() {
    assert!(compile("import square from \"./math.atto\"; pub fnc main(): int {return square(7); }").is_err());
}

#[test]
fn grouped_relative_function_imports_use_production_typed_linker() {
    let dep = "fnc twice(int x): int {return x+x;} pub fnc square(int x): int {return x*x;} pub fnc add(int x): int {return twice(x);}";
    let legacy = wasm("import square as calc, add from './math.atto'; pub fnc main(): int {return calc(6)+add(3);}", dep);
    for clause in ["import fnc [square as calc, add] from './math.atto';",
                   "import fnc (square as calc, add) from './math.atto';",
                   "import [fnc square as calc, fnc add] from './math.atto';",
                   "import ([fnc square as calc, add]) from './math.atto';"] {
        let src = format!("{clause} pub fnc main(): int {{return calc(6)+add(3);}}");
        assert_eq!(wasm(&src, dep), legacy, "{clause}");
    }
}
#[test]
fn typed_local_groups_enforce_public_exports_and_no_implicit_binding() {
    let dep = "fnc hidden(int x): int {return x;} pub fnc square(int x): int {return hidden(x)*x;}";
    assert_eq!(code("import fnc [square, hidden] from './math.atto'; pub fnc main(): int {return square(6);}", dep), "E_PRIVATE");
    assert_eq!(code("import fnc [square, missing] from './math.atto'; pub fnc main(): int {return square(6);}", dep), "E_UNBOUND");
    assert!(build("import fnc [square as calc] from './math.atto'; pub fnc main(): int {return square(6);}", dep).is_err());
    assert!(compile("import fnc [square] from './math.atto'; pub fnc main(): int {return square(6);}").is_err());
}
#[test]
fn grouped_relative_preflight_denies_invalid_later_clauses_before_io() {
    for root in [
        "import fnc [square] from './math.atto'; import fnc [square] from './math.atto';",
        "import fnc [square] from './math.atto'; import class [C] from './math.atto';",
        "import fnc [square] from './math.atto'; import fnc [x] from pkg '@a/b/c';",
        "import fnc [square] from './math.atto'; import fnc [x] from '../../escape.atto';",
    ] {
        let mut loads = 0;
        let src = format!("{root} pub fnc main(): int {{return 42;}}");
        assert!(compile_with_imports(&src, Path::new("project/main.atto"), |_| {
            loads += 1; Ok(MATH.into())
        }).is_err(), "{root}");
        assert_eq!(loads, 0, "{root}");
    }
}
