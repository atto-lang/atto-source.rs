//! Compiler-executed admission boundary for typed failure operations.
//! This does not claim that source interfaces or raise/panic are implemented.
use atto_compiler::compile;

fn source(action: &str) -> String {
    format!("fnc fail(): int {{ {action} }}\npub fnc main(): int {{ return 0; }}")
}

#[test]
fn built_in_error_is_currently_the_only_admitted_throw_payload() {
    let bytes = compile(&source("throw Error(\"guest failure\");"))
        .expect("built-in Error plus implicit Throwable is executable");
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");

    for action in [
        "throw 42;",
        "throw false;",
        "throw \"bad\";",
        "throw new Unrelated();",
    ] {
        let snippet = if action.contains("Unrelated") {
            format!("define class Unrelated as\nend\n{}", source(action))
        } else {
            source(action)
        };
        let err = compile(&snippet).expect_err(action);
        assert_eq!(err.code, "E_TYPE", "{action}: {err}");
    }
}

#[test]
fn unimplemented_raise_panic_and_effect_clauses_never_emit_a_guest() {
    for action in [
        "raise Error(\"escalated\");",
        "panic Error(\"fatal\");",
    ] {
        let err = compile(&source(action)).expect_err(action);
        assert_eq!(err.code, "E_UNSUPPORTED", "{action}: {err}");
    }
    for clause in ["throws(Error)", "raises(Error)", "panics(Error)"] {
        let text = format!(
            "fnc no_failure(): int {clause} {{ return 1; }}\npub fnc main(): int {{ return no_failure(); }}"
        );
        let wasm = compile(&text).expect("bounded signature with no failure must compile");
        assert_eq!(&wasm[..8], b"\\0asm\\x01\\0\\0\\0", "{clause}");
    }
}
