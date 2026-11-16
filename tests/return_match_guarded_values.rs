//! Source compiler tests for guarded match bindings and braced return arms.
//! Actual execution of exact public match-values.atto is required by the
//! wasm32-vs-native browser guest conformance audit.
use atto_compiler::compile;

fn compiled(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("unexpected compile error: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "must produce genuine Wasm");
    assert_eq!(wasm, compile(source).unwrap(), "deterministic lowering");
}
fn diagnostic(source: &str) -> String {
    compile(source).expect_err("must fail without a guest artifact").code.to_string()
}

#[test]
fn exact_published_match_values_preview_compiles() {
    compiled(include_str!("playground_previews/match-values.atto"));
}

#[test]
fn guarded_binding_and_braced_returns_are_source_compiled() {
    compiled(r#"
        pub fnc main(): int {
            return match 7 over
                on n when n > 10 -> { return 1; }
                on n when n > 5 -> { return n; }
                on _ -> { return 0; }
            end;
        }
    "#);
    compiled(r#"
        pub fnc main(): int {
            return match 7 over
                on 0 -> 0,
                on n when n > 3 -> n,
                on _ -> -1,
            end;
        }
    "#);
    compiled(r#"
        pub fnc main(): int {
            return match 7 over
                on 7 -> { if true { return 11; } else { return 12; } fi }
                on _ -> 0,
            end;
        }
    "#);
}

#[test]
fn guarded_binders_do_not_cover_the_unmatched_domain() {
    assert_eq!(diagnostic(r#"
        pub fnc main(): int {
            return match 7 over on n when n > 0 -> n, end;
        }
    "#), "E_MATCH");
}

#[test]
fn reject_wrong_guard_type_and_fallthrough_block() {
    assert_eq!(diagnostic(r#"
        pub fnc main(): int {
            return match 7 over on n when 5 -> n, on _ -> 0, end;
        }
    "#), "E_TYPE");
    assert_eq!(diagnostic(r#"
        pub fnc main(): int {
            return match 7 over on 7 -> { val x = 5; } on _ -> 0, end;
        }
    "#), "E_RETURN");
    assert_eq!(diagnostic(r#"
        pub fnc main(): int {
            return match 7 over on 0 -> 0 on _ -> 1, end;
        }
    "#), "E_SYNTAX");
}
