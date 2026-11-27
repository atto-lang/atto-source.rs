//! Guard + statement-position caret + directional Future capture regression.
//! These tests invoke the actual Atto parser/type-checker/Wasm compiler.
//! Runtime branch outcomes and JSPI yield counts are tested in
//! tests/browser-async-e2e.mjs; merely seeing the Wasm magic is not execution.
use atto_compiler::compile;

fn compiled(source: &str) -> Vec<u8> {
    let wasm = compile(source).unwrap_or_else(|err| panic!("expected compiled Wasm: {err}\n{source}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).unwrap(), "nondeterministic Wasm: {source}");
    wasm
}

fn rejected(source: &str, code: &str) {
    let diag = compile(source).expect_err("rejected input must produce no guest artifact");
    assert_eq!(diag.code, code, "{source}: {diag}");
}

#[test]
fn caret_and_return_have_identical_guard_exit_semantics() {
    for spelling in ["^", "return"] {
        for condition in ["true", "false"] {
            let source = format!(
                "pub fnc main(): int {{ guard {condition} {{ {spelling} -1; }} {spelling} 42; }}"
            );
            compiled(&source);
        }
        compiled(&format!(r#"
            pub fnc main(): int {{
                guard false {{
                    if true {{ {spelling} 1; }} else {{ {spelling} 2; }} fi
                }}
                {spelling} 3;
            }}
        "#));
        compiled(&format!(r#"
            fnc ready(): bool {{ stdio.println("checked"); {spelling} true; }}
            pub fnc main(): int {{
                guard ready() {{ {spelling} -1; }}
                {spelling} 42;
            }}
        "#));
    }
}

#[test]
fn guard_after_await_transform_capture_checks_and_returns_a_value() {
    for spelling in ["susp", "suspends"] {
        let source = format!(r#"
            fnc transform_result(int x): int {{ ^ x + 1; }}
            pub suspends fnc main(): int {{
                val pending = task {{ return 41; }};
                pending
                    |> {spelling} |f| -> await f
                    |> transform_result
                    >> val result: int;
                guard result > 0 {{
                    ^ -1;
                }}
                ^ result;
            }}
        "#);
        let wasm = compiled(&source);
        assert!(wasm.windows(10).any(|w| w == b"task_await"));
        assert!(wasm.windows(9).any(|w| w == b"cooperate"));
    }
}

#[test]
fn guard_before_task_creation_and_guard_failure_await_return_are_both_accepted() {
    compiled(r#"
        pub suspends fnc main(): int {
            guard false { ^ -1; }
            val pending = task { return 41; };
            ^ pending |> susp |f| -> await f >> val answer: int;
        }
    "#);
    for spelling in ["^", "return"] {
        let source = format!(r#"
            fnc transform_result(int x): int {{ ^ x + 1; }}
            pub suspends fnc main(): int {{
                val pending = task {{ return 41; }};
                guard false {{
                    {spelling} pending
                        |> susp |f| -> await f
                        |> transform_result
                        >> val answer: int;
                }}
                ^ 0;
            }}
        "#);
        let wasm = compiled(&source);
        assert!(wasm.windows(10).any(|w| w == b"task_await"),
            "failure branch must preserve the real await effect");
    }
}

#[test]
fn guard_failure_paths_must_exit_even_with_caret_and_nested_conditionals() {
    for source in [
        "pub fnc main(): int { guard true { val x = 1; } ^ 42; }",
        "pub fnc main(): int { guard true { if true { ^ 1; } fi } ^ 42; }",
        "pub fnc main(): int { guard true { if true { ^ 1; } else { val x = 2; } fi } ^ 42; }",
        "pub fnc main(): int { guard true { block { val x = 1; } } ^ 42; }",
        "pub fnc main(): int { guard true { while true { ^ 1; } } ^ 42; }",
    ] {
        rejected(source, "E_GUARD");
    }
}

#[test]
fn guard_uses_bool_and_forbids_swift_else_with_specific_diagnostics() {
    rejected("pub fnc main(): int { guard 41 { ^ 0; } ^ 42; }", "E_TYPE");
    rejected("pub fnc main(): int { guard false { ^ 1; } else { ^ 2; } ^ 42; }",
             "E_GUARD_ELSE_NOT_REQUIRED");
    rejected("pub fnc main(): int { guard false { return 1; } else { return 2; } return 42; }",
             "E_GUARD_ELSE_NOT_REQUIRED");
}

#[test]
fn guard_failure_caret_pipeline_enforces_effect_types_and_terminal_capture() {
    rejected(r#"
        pub fnc main(): int {
            val p = task { return 41; };
            guard false { ^ p |> susp |f| -> await f >> val answer: int; }
            ^ 0;
        }
    "#, "E_EFFECT");
    rejected(r#"
        pub susp fnc main(): int {
            val p = task { return 41; };
            guard false { ^ p |> susp |f| -> await f >> val answer: string; }
            ^ 0;
        }
    "#, "E_TYPE");
    rejected(r#"
        pub susp fnc main(): int {
            val p = task { return 41; };
            guard false { ^ p |> susp |f| -> await f >> const answer: int; }
            ^ 0;
        }
    "#, "E_CONST_EFFECT");
    rejected(r#"
        pub susp fnc main(): int {
            val p = task { return 41; };
            guard false { ^ p >> val answer: int; }
            ^ 0;
        }
    "#, "E_IO_SYNTAX");
}

#[test]
fn guard_return_cannot_silently_escape_borrow_check_during_await() {
    rejected(r#"
        define class Parcel as
            pub let int value = 1;
        end
        pub suspends fnc main(): int {
            let parcel = new Parcel();
            val loan = borrow! parcel;
            val pending = task { return 41; };
            guard false {
                ^ pending |> susp |f| -> await f >> val answer: int;
            }
            ^ loan.value;
        }
    "#, "E_BORROW");
}

#[test]
fn guard_and_caret_cannot_escape_a_critical_write_lease() {
    // Neither caret return nor an else-free guard may bypass the nonescaping
    // critical state lease contract. The parser must reject before guest Wasm.
    for forbidden in ["^;", "guard false { ^; }", "await task { return 42; };"] {
        let src = format!(r#"
define state container Counter as
    state {{ let int count: 0; }}
    pub bad(): void {{
        handle_state(critical |s| -> {{
            {forbidden}
            s.count = 1;
        }});
    }}
end
pub fnc main(): int {{ ^ 0; }}
"#);
        rejected(&src, "E_STATE_BORROW");
    }
}
