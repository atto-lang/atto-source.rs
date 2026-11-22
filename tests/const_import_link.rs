//! Bounded executable relative-file scalar constant import regression tests.
//! A successful `compile_with_imports` returns real compiled Wasm; guest
//! invocation is separately exercised by constant_import_guest.mjs.
use atto_compiler::{compile_with_imports, Diagnostic};
use std::{cell::Cell, io, path::Path};

fn compile(root: &str, file: &str) -> Result<Vec<u8>, Diagnostic> {
    compile_with_imports(root, Path::new("project/main.atto"), |path| {
        if path == Path::new("project/config.atto") { Ok(file.to_owned()) }
        else { Err(io::Error::new(io::ErrorKind::NotFound, "missing")) }
    })
}
fn error(root: &str, file: &str) -> &'static str {
    let e = compile(root, file).expect_err("rejected program must emit no Wasm");
    assert!(e.pos.line > 0, "diagnostics must retain source locations");
    e.code
}
fn valid(root: &str, file: &str) {
    let bytes = compile(root, file).expect("linked compiler must emit guest Wasm");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(root, file).unwrap(), "deterministic guest");
}

#[test]
fn int_bool_and_string_literals_link_with_normal_typecheck() {
    valid(
        r#"import const status_valid from "./config.atto";
            pub fnc main(): int { return status_valid; }"#,
        "define pub const int status_valid = 200;",
    );
    valid(
        r#"import const flag from "./config.atto";
            pub fnc main(): int { if flag then return 1; else return 0; fi }"#,
        "define pub const bool flag = true;",
    );
    valid(
        r#"import const state from "./config.atto";
            pub fnc main(): int { if state eq "ready" then return 42; else return 0; fi }"#,
        r#"define pub const string state as "ready";"#,
    );
    valid(
        r#"import const status_valid as ok from "./config.atto";
            pub fnc main(): int { return ok; }"#,
        "define pub const int status_valid = 200;",
    );
}

#[test]
fn multiple_imports_from_same_file_resolve_once() {
    let reads = Cell::new(0);
    let result = compile_with_imports(
        r#"import const left from "./config.atto";
            import const right from "./config.atto";
            pub fnc main(): int { return left + right; }"#,
        Path::new("project/main.atto"),
        |path| {
            assert_eq!(path, Path::new("project/config.atto"));
            reads.set(reads.get() + 1);
            Ok("define pub const int left = 19; define pub const int right = 23;".to_owned())
        }
    );
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(reads.get(), 1, "one source load per logical file identity");
}

#[test]
fn public_private_and_unknown_exports_are_distinguished() {
    let file = "define const int internal_key = 11; define pub const int exported = 22;";
    assert_eq!(error("import const internal_key from \"./config.atto\"; pub fnc main(): int {return 0;}",file),"E_PRIVATE");
    assert_eq!(error("import const missing from \"./config.atto\"; pub fnc main(): int {return 0;}",file),"E_UNBOUND");
    assert_eq!(error("import const exported as x, exported as x from \"./config.atto\"; pub fnc main(): int {return x;}",file),"E_DUPLICATE");
}

#[test]
fn imported_constant_cannot_be_written_or_used_as_callable() {
    let file = "define pub const int status_valid = 200;";
    assert_eq!(error("import const status_valid from \"./config.atto\"; pub fnc main(): int {status_valid = 4; return 0;}",file),"E_ASSIGN_CONST");
    assert_eq!(error("import const status_valid from \"./config.atto\"; pub fnc main(): int {return status_valid();}",file),"E_IMPORT_KIND");
    assert_eq!(error("import const status_valid from \"./config.atto\"; pub fnc status_valid(): int {return 0;} pub fnc main(): int {return 0;}",file),"E_DUPLICATE");
}

#[test]
fn source_size_path_type_and_runtime_effects_fail_closed() {
    assert_eq!(error("import const x from \"../../escape.atto\"; pub fnc main(): int {return x;}",
        "define pub const int x = 1;"), "E_IMPORT_PATH");
    assert_eq!(error("import const x from \"./config.atto\"; pub fnc main(): int {return x;}",
        "define pub const int x = \"wrong\";"), "E_CONST_TYPE");
    assert_eq!(error("import const x from \"./config.atto\"; pub fnc main(): int {return 0;}",
        "define pub const string x = std.env.read_env(\"X\");"), "E_UNSUPPORTED");
    assert_eq!(error("import const x from \"./config.atto\"; pub fnc main(): int {return 0;}",
        "define pub const int x = get_value();"), "E_UNSUPPORTED");
    assert_eq!(error("import const x from pkg \"std.http\"; pub fnc main(): int {return 0;}",
        "define pub const int x = 1;"), "E_UNSUPPORTED");
}

#[test]
fn malformed_source_never_publishes_fallback_artifact() {
    for source in [
        "import const * as c from \"./config.atto\"; pub fnc main(): int {return 0;}",
        "import const x, from \"./config.atto\"; pub fnc main(): int {return 0;}",
        "import const x from \"./config.atto\" pub fnc main(): int {return 0;}",
    ] {
        assert!(compile(source, "define pub const int x = 1;").is_err());
    }
}

#[test]
fn forged_compiler_import_identifiers_fail_closed() {
    let file = "define pub const int status_valid = 200;";
    assert_eq!(
        error(
            "import const status_valid from \"./config.atto\"; pub fnc main(): int {return __atto_file_import_0.status_valid;}",
            file,
        ),
        "E_RESERVED"
    );
    assert_eq!(
        error(
            "import const status_valid from \"./config.atto\"; pub fnc main(): int {return 0;}",
            "define pub const int __atto_internal_module_0 = 200; define pub const int status_valid = 200;",
        ),
        "E_RESERVED"
    );
}

#[test]
fn namespaced_relative_const_provider_with_unused_functions_is_linkable() {
    valid(
        "import const status_valid from \"./config.atto\"; pub fnc main(): int { return status_valid; }",
        "use namespace std.http; pub fnc unrelated(): int { return 3; } define pub const int status_valid = 200;",
    );
}

#[test]
fn exporting_a_function_is_not_a_constant_even_if_public() {
    assert_eq!(
        error(
            "import const status_valid from \"./config.atto\"; pub fnc main(): int { return 0; }",
            "pub fnc status_valid(): int { return 200; }",
        ),
        "E_IMPORT_KIND"
    );
    assert_eq!(
        error(
            "import const hidden from \"./config.atto\"; pub fnc main(): int { return 0; }",
            "fnc hidden(): int { return 200; }",
        ),
        "E_PRIVATE"
    );
}
