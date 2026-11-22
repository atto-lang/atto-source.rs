//! Exact guard+caret integration, re-based on current main.
//! These checks must run in a real Cargo job; source presence is not a pass.
use atto_compiler::compile;

fn accepts(src: &str) {
    let wasm = compile(src).unwrap_or_else(|e| panic!("expected valid program: {e}\n{src}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "must produce guest Wasm");
}
fn rejects(src: &str, code: &str) {
    let err = compile(src).expect_err("invalid source must not emit Wasm");
    assert_eq!(err.code, code, "{src}: {err}");
}
fn entry() -> &'static str {
    "pub fnc main(): int { process_explicit(); ^ 42; }"
}

#[test]
fn exact_keyword_and_brace_guards_in_arrow_void_end_callable() {
    for function in [
        "fnc process_explicit() -> void\n    guard false else ^ void end\nend",
        "fnc process_explicit() -> void\n    guard false { ^ void }\nend",
        "fnc process_explicit() -> void\n    guard true else ^ void end\nend",
        "fnc process_explicit() -> void\n    guard true { ^ void }\nend",
        "fnc process_explicit() -> void\n    guard false else return void end\nend",
        "fnc process_explicit() -> void\n    guard false { return void }\nend",
        "fnc process_explicit() -> void\n    guard false else ^ end\nend",
        "fnc process_explicit() -> void\n    guard false { ^ }\nend",
    ] {
        accepts(&format!("{function}\n{}", entry()));
    }
}

#[test]
fn caret_return_and_if_grammar_remain_compatible() {
    for body in [
        "if true { ^ void; }",
        "if true { ^ void; } fi",
        "if true then ^ void fi",
        "do if true then ^ void fi",
        "if true then if true { ^ void; } fi",
        "if true then if true { ^ void; } fi fi",
        "if true { if true then ^ void fi }",
    ] {
        accepts(&format!("fnc process_explicit() -> void {{ {body} }} {}", entry()));
    }
}

#[test]
fn guard_must_not_consume_outer_else_or_fi() {
    accepts("pub fnc main(): int { if true then guard true { ^ 7; } else ^ 9; fi ^ 42; }");
    accepts("pub fnc main(): int { if true then guard true else ^ 7; end else ^ 9; fi ^ 42; }");
    rejects(
        "pub fnc main(): int { guard true { ^ 7; } else ^ 9; end ^ 42; }",
        "E_GUARD_ELSE_NOT_REQUIRED"
    );
}

#[test]
fn rejects_invalid_returns_and_fallthrough() {
    rejects("pub fnc main(): int { ^ void; }", "E_RETURN");
    rejects("pub fnc main(): int { return void; }", "E_RETURN");
    for failure in ["guard false else ^ void end", "guard false { ^ void }"] {
        let src = format!("fnc process_explicit() -> int\n    {failure}\nend\npub fnc main(): int {{ ^ process_explicit(); }}");
        rejects(&src, "E_RETURN");
    }
    for failure in ["guard false else val x = 1; end", "guard false { val x = 1; }"] {
        let src = format!("fnc process_explicit() -> void\n    {failure}\nend\n{}", entry());
        rejects(&src, "E_GUARD");
    }
}

#[test]
fn guard_rejects_nonreturn_escapes_even_before_return() {
    for failure in [
        "throw Error(\"bad\");",
        "break;",
        "continue;",
        "throw Error(\"bad\"); ^ void;",
    ] {
        for keyword in [true, false] {
            let guard = if keyword {
                format!("guard false else {failure} end")
            } else {
                format!("guard false {{ {failure} }}")
            };
            let src = format!("fnc process_explicit() -> void {{ {guard} }} {}", entry());
            rejects(&src, "E_GUARD");
        }
    }
}

#[test]
fn all_invalid_void_marker_expressions_fail_closed() {
    for src in [
        "fnc process_explicit() -> void { ^ void + 1; } pub fnc main(): int { ^ 42; }",
        "fnc process_explicit() -> void { ^ void 42; } pub fnc main(): int { ^ 42; }",
        "pub fnc main(): int { val x = void; ^ 42; }",
    ] {
        compile(src).expect_err("void is not a runtime value");
    }
}


#[test]
fn nested_nonreturn_escapes_cannot_hide_before_caret_void() {
    for failure in [
        "while true { throw Error(\"bad\"); } ^ void;",
        "loop { raise Error(\"bad\"); } ^ void;",
        "try { throw Error(\"bad\"); } catch (Error e) { ^ void; } ^ void;",
    ] {
        let source = format!(
            "fnc process_explicit() -> void {{ guard false else {failure} end }} {}",
            entry()
        );
        rejects(&source, "E_GUARD");
    }
}


#[test]
fn multiple_void_guards_are_explicit_end_statements() {
    for function in [
        "fnc process_explicit() -> void\n    guard true else ^ void end\n    guard false { ^ void }\nend",
        "fnc process_explicit() -> void\n    guard true { ^ void }\n    guard false else ^ void end\nend",
        "fnc process_explicit() -> void\n    val x = 1;\n    guard false else ^ void end\nend",
        "fnc process_explicit() -> void\n    guard true else ^ void end\n    val x = 1;\nend",
        "fnc process_explicit() -> void\n    guard true { ^ void }\nend",
    ] {
        accepts(&format!("{function}\n{}", entry()));
    }
}

#[test]
fn multiple_nonvoid_guards_without_success_return_still_reject() {
    for function in [
        "fnc process_explicit() -> int\n    guard true else ^ 2 end\n    guard false { ^ 3 }\nend",
        "fnc process_explicit() -> int\n    guard false else ^ 2 end\nend",
    ] {
        let source = format!("{function}\npub fnc main(): int {{ ^ process_explicit(); }}");
        compile(&source).expect_err("nonvoid function must not silently fall through");
    }
}


#[test]
fn guard_lazy_boolean_canaries_compile_in_both_styles() {
    for prefix in [
        "guard false, overflow_if_called(9223372036854775807)",
        "guard true | overflow_if_called(9223372036854775807)",
    ] {
        for end in [
            "else ^ void end",
            "{ ^ void }",
        ] {
            let src=format!(
                "fnc overflow_if_called(int v): bool {{ return v + 1 > 0; }}\n\
                 fnc process_explicit() -> void\n    {prefix} {end}\nend\n{}",
                entry()
            );
            accepts(&src);
        }
    }
}

#[test]
fn guard_requires_strict_boolean_predicate() {
    for failure in ["guard 7 else ^ void end", "guard 7 { ^ void }"] {
        let src=format!("fnc process_explicit() -> void\n    {failure}\nend\n{}",entry());
        rejects(&src,"E_TYPE");
    }
}
