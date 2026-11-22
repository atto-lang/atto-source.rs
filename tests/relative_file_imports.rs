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
fn compiler_owned_scalar_library_never_calls_host_loader() {
    let entry = "import fnc equal_int, in_range_int from \"std/testing_scalar\"; \
        pub fnc main(): int { return in_range_int(42,40,50) + equal_int(7,7); }";
    let native = compile(entry).expect("single-source importer must resolve compiler-owned .atto");
    assert!(native.starts_with(b"\0asm\x01\0\0\0"));
    let linked = compile_with_imports(entry, Path::new("src/main.atto"), |_| {
        panic!("compiler-owned standard library must not use ambient file loader")
    }).expect("compiler-owned import must bypass project resolver");
    assert!(linked.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(native, linked, "compiler-local bytes must be independent of project path");
}
#[test]
fn compiler_owned_scalar_alias_and_namespace_imports_compile() {
    for entry in [
        "import fnc greater_than_int as greater from \"atto.std/testing_scalar.atto\"; \
         pub fnc main(): int { return greater(42,41); }",
        "import * as Testing from \"std/testing_scalar.atto\"; \
         pub fnc main(): int { return Testing.less_than_int(41,42); }",
        "import fnc in_range_int from \"std/testing_scalar.atto\"; \
         pub fnc main(): int { return in_range_int(-9223372036854775808,-9223372036854775808,9223372036854775807); }",
    ] {
        assert!(compile(entry).is_ok(), "compiler-local import must compile: {entry}");
    }
}
#[test]
fn compiler_owned_scalar_import_boundaries_are_fail_closed() {
    for (entry, expected) in [
        ("import fnc equal_int from \"std/testing_scalar.ores\"; pub fnc main():int{return 0;}", "E_UNSUPPORTED"),
        ("import fnc equal_int from \"std/../testing_scalar\"; pub fnc main():int{return 0;}", "E_UNSUPPORTED"),
        ("import fnc equal_int from \"std/unknown\"; pub fnc main():int{return 0;}", "E_UNSUPPORTED"),
        ("import fnc missing from \"std/testing_scalar\"; pub fnc main():int{return missing(1,1);}", "E_UNBOUND"),
        ("import fnc equal_int as alias, in_range_int as alias from \"std/testing_scalar\"; pub fnc main():int{return 0;}", "E_DUPLICATE"),
        ("import fnc equal_int from \"std/testing_scalar\"; pub fnc equal_int(int x,int y):int{return 1;} pub fnc main():int{return 0;}", "E_DUPLICATE"),
        ("import fnc equal_int from \"std/testing_scalar\"; pub fnc main():int{return __atto_file_import_0.equal_int(1,1);}", "E_RESERVED"),
        ("import fnc equal_int from \"./stdlib/testing_scalar.atto\"; pub fnc main():int{return 0;}", "E_IMPORT"),
    ] {
        let err = compile(entry).expect_err("invalid library import must never emit Wasm");
        assert_eq!(err.code, expected, "{entry}: unexpected compiler diagnostic");
    }
}
