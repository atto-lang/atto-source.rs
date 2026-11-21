//! Source-compiled guard-v1 conformance. Guest execution of the exact pinned
//! website source is enforced independently by playground-preview-audit.mjs.
use atto_compiler::compile;

fn compile_ok(source: &str) {
    let bytes = compile(source).unwrap_or_else(|err| panic!("expected compiled guest: {err}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "genuine Wasm guest");
    assert_eq!(bytes, compile(source).unwrap(), "deterministic lowering");
}

fn error_code(source: &str) -> String {
    compile(source).expect_err("must reject without emitting a guest").code.to_string()
}

#[test]
fn exact_website_guard_preview_compiles() {
    compile_ok(include_str!("playground_previews/guard.atto"));
}

#[test]
fn terminal_guard_lowers_to_typed_boolean_branch() {
    compile_ok(
        r#"
        pub fnc main(): int {
            let int value = 7;
            guard value > 0 { return 1; }
            return 2;
        }
    "#,
    );
    compile_ok(
        r#"
        pub fnc main(): int {
            guard false {
                if true { return 1; } else { return 2; } fi
            }
            return 3;
        }
    "#,
    );
    compile_ok(
        r#"
        pub fnc main(): int {
            guard true { throw Error("failure"); }
            return 42;
        }
    "#,
    );
}

#[test]
fn guard_requires_terminal_failure_branch_on_all_paths() {
    for source in [
        r#"pub fnc main(): int { guard true { } return 0; }"#,
        r#"pub fnc main(): int { guard true { val x = 1; } return 0; }"#,
        r#"pub fnc main(): int { guard true { if true { return 1; } fi } return 0; }"#,
        r#"pub fnc main(): int { guard true { if true { return 1; } else { val x = 2; } fi } return 0; }"#,
        r#"pub fnc main(): int { guard true { while true { return 1; } } return 0; }"#,
    ] {
        assert_eq!(error_code(source), "E_GUARD", "{source}");
    }
}

#[test]
fn guard_rejects_non_boolean_malformed_and_else_forms() {
    assert_eq!(error_code(r#"pub fnc main(): int { guard 7 { return 1; } return 0; }"#), "E_TYPE");
    assert_eq!(error_code(r#"pub fnc main(): int { guard false return 1; }"#), "E_GUARD");
    assert!(compile(r#"pub fnc main(): int { guard true { return 1; } else { return 2; } return 3; }"#).is_err());
    assert!(compile(r#"pub fnc main(): int { guard = 1 { return 1; } return 2; }"#).is_err());
}
