//! Source-compiled parity for guard's equivalent brace and else/end forms.
//! These test real compiler admission into Wasm; guest execution and formatter
//! round-trips need separate exact-head runtime/format evidence.
use atto_compiler::compile;

fn accepted(source: &str) {
    let output = compile(source).unwrap_or_else(|e| panic!("expected accepted guard: {e}\n{source}"));
    assert!(output.starts_with(b"\0asm\x01\0\0\0"), "compiler emitted Wasm");
}

fn rejected(source: &str, code: &str) {
    let diag = compile(source).expect_err("invalid guard must not produce Wasm");
    assert_eq!(diag.code, code, "{source}: {diag}");
}

#[test]
fn both_spelling_forms_accept_identical_continue_and_failure_branches() {
    for (predicate, failure) in [("true", "return 0;"), ("false", "return 0;")] {
        accepted(&format!("pub fnc main(): int {{ guard {predicate} {{ {failure} }} return 42; }}"));
        accepted(&format!("pub fnc main(): int {{ guard {predicate} else {failure} end return 42; }}"));
    }
}

#[test]
fn both_forms_accept_nested_exhaustive_returns() {
    accepted(r#"pub fnc main(): int {
        guard true { if true { return 0; } else { return 1; } fi }
        return 42;
    }"#);
    accepted(r#"pub fnc main(): int {
        guard true else
            if true then
                return 0;
            else
                return 1;
            fi
        end
        return 42;
    }"#);
}

#[test]
fn keyword_guard_ends_before_outer_if_else_and_supports_nested_guard() {
    accepted(r#"pub fnc main(): int {
        if true then
            guard true else
                guard false else
                    return 1;
                end
                return 2;
            end
        else
            return 3;
        fi
        return 42;
    }"#);
}

#[test]
fn keyword_guard_checks_boolean_type_and_proves_termination() {
    rejected("pub fnc main(): int { guard 41 else return 0; end return 42; }", "E_TYPE");
    rejected("pub fnc main(): int { guard true else val x = 1; end return 42; }", "E_GUARD");
    rejected("pub fnc main(): int { guard true else if true { return 0; } fi end return 42; }", "E_GUARD");
    assert!(compile("pub fnc main(): int { guard true else return 0; fi return 42; }").is_err());
}

#[test]
fn never_accept_mixed_guard_delimiters_or_nonterminal_braced_guard() {
    rejected("pub fnc main(): int { guard true { return 0; } else return 1; end return 42; }",
             "E_GUARD_ELSE_NOT_REQUIRED");
    rejected("pub fnc main(): int { guard true { val x = 1; } return 42; }", "E_GUARD");
    rejected("pub fnc main(): int { guard true return 1; }", "E_GUARD");
}
