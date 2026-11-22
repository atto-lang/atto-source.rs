//! Production namespace-aware relative .atto import regression.
//! The real compiler emits Wasm or fails with an exact diagnostic; these
//! source tests do not claim guest execution or actor-runtime authority.
use atto_compiler::{compile_with_imports, compile_with_path};
use std::io;
use std::path::Path;

const MAIN_FILE: &str = "project/src/main.atto";
const HELPER_FILE: &str = "project/lib/math.atto";
const MAIN: &str = r#"use namespace demo.app;
import twice from "../lib/math.atto";
pub fnc main(): int { return twice(21); }"#;
const HELPER: &str = r#"use namespace demo.lib;
pub fnc twice(int x): int { return x + x; }"#;

fn build(root: &str, helper: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_with_imports(root, Path::new(MAIN_FILE), |path| {
        if path == Path::new(HELPER_FILE) {
            Ok(helper.to_string())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "unapproved helper import"))
        }
    })
}
fn error_code(root: &str, helper: &str) -> &'static str {
    let failure = build(root, helper).expect_err("forbidden namespace/import must fail before artifact");
    failure.code
}

#[test]
fn namespace_first_imports_produce_real_wasm_and_recompile_deterministically() {
    let wasm = build(MAIN, HELPER).expect("namespace headers on entry and imported file");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "real compiler Wasm artifact");
    assert_eq!(wasm, build(MAIN, HELPER).unwrap(), "deterministic exact-source artifact");
}

#[test]
fn namespace_first_and_derived_headers_do_not_create_missing_import_authority() {
    let rooted = MAIN.replace("use namespace demo.app;", "use default namespace;");
    let helper = HELPER.replace("use namespace demo.lib;", "use default namespace;");
    assert!(build(&rooted, &helper).is_ok(), "relative package paths derive file namespaces");
    let missing = rooted.replace("import twice from \"../lib/math.atto\";", "");
    assert!(build(&missing, &helper).is_err(), "a namespace alone cannot import a helper");
}

#[test]
fn namespace_imports_preserve_public_private_and_file_boundary() {
    assert_eq!(error_code(MAIN, &HELPER.replace("pub fnc twice", "fnc twice")), "E_PRIVATE");
    let illegal = MAIN.replace("../lib/math.atto", "../../../secret.atto");
    assert_eq!(error_code(&illegal, HELPER), "E_IMPORT_PATH");
    assert_eq!(
        error_code(MAIN, "use namespace demo.lib; import secret from \"./secret.atto\"; pub fnc twice(int x): int { return x+x; }"),
        "E_UNSUPPORTED",
        "only one-hop imports are admitted"
    );
    assert_eq!(
        error_code(MAIN, "use namespace demo.lib; pub fnc __atto_file_import_0(int x): int { return x; } pub fnc twice(int x): int { return x + x; }"),
        "E_RESERVED",
        "guest namespace cannot mint compiler-reserved identity"
    );
}

#[test]
fn malformed_or_misplaced_namespace_headers_fail_closed() {
    assert_eq!(
        error_code(MAIN, "use namespace demo.lib; use namespace stolen; pub fnc twice(int x): int { return x+x; }"),
        "E_DUPLICATE"
    );
    assert_eq!(
        error_code(MAIN, "pub fnc twice(int x): int { return x+x; } use namespace stolen;"),
        "E_SYNTAX"
    );
    assert_eq!(
        error_code(&MAIN.replace("use namespace demo.app;", "use namespace demo.app; use namespace stolen;"), HELPER),
        "E_DUPLICATE"
    );
    assert_eq!(
        error_code(&MAIN.replace("use namespace demo.app;", "").replace(
            "pub fnc main():", "use namespace misplaced; pub fnc main():"
        ), HELPER),
        "E_SYNTAX"
    );
}

#[test]
fn standalone_namespaced_pure_function_is_real_guest_not_syntax_only() {
    let src = "use namespace std.actor.helpers; pub fnc main(): int { return 42; }";
    let wasm = compile_with_path(src, Path::new("project/src/standalone.atto"))
        .expect("valid namespace should not force syntax-only actor receiver mode");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile_with_path(src, Path::new("project/src/standalone.atto")).unwrap());
}

#[test]
fn standalone_namespace_cannot_forge_an_unimported_policy_function() {
    let no_import = r#"use namespace std.actor.consumer;
pub fnc main(): int {
  if actor_mailbox_admissible(true, true, true, true, 0, 1) then
    return 1;
  fi
  return 42;
}"#;
    let err = compile_with_path(no_import, Path::new("project/src/standalone.atto"))
        .expect_err("namespace alone must not bind an actor policy function");
    assert_eq!(err.code, "E_UNBOUND", "must reject unbound import, not a syntax-only fallback");
    let nominal = "use namespace std.actor; declare actor Supervisor; pub fnc main(): int { return 42; }";
    assert!(compile_with_path(nominal, Path::new("project/src/standalone.atto")).is_err(),
        "actor declaration is not a permitted pure-function lowering");
}
