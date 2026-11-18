//! Failure-effect rows: explicit empty bounds and retained nominal names.
//! Browser execution currently admits only core Error as a failure type.
use atto_compiler::{compile, lexer, parser::Parser};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(src).expect("lex")).parse_program().expect("parse")
}
fn wasm(src: &str) {
    let bytes = compile(src).unwrap_or_else(|e| panic!("unexpected diagnostic {e} for: {src}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(bytes, compile(src).unwrap(), "Wasm must be byte-deterministic");
}
fn denied(src: &str, code: &str) {
    let err = compile(src).expect_err("effect violation must not issue a Wasm artifact");
    assert_eq!(err.code, code, "{src}: {err}");
}

#[test]
fn explicit_empty_rows_are_not_undeclared_and_keep_nominal_type_sets() {
    for spell in ["@Throws()", "@throws()", "@Raises()", "@Panics()"] {
        let src = format!("{spell}\npub fnc main(): int {{ return 42; }}");
        let p = parse(&src);
        let effects = &p.funcs[0].failure_effects;
        assert!(effects.explicit, "{spell}");
        assert_eq!(effects.mask(), 0, "{spell}");
        assert_eq!(effects.declared.iter().filter(|v| **v).count(), 1);
        assert!(effects.throws.is_empty() && effects.raises.is_empty() && effects.panics.is_empty());
        wasm(&src);
    }
    for spell in ["throws()", "raises()", "panics()"] {
        wasm(&format!("pub fnc main(): int {spell} {{ return 42; }}"));
    }
    let p = parse("@Throws(Error)\n@Raises()\npub fnc main(): int {return 42;}");
    let e = &p.funcs[0].failure_effects;
    assert_eq!(e.throws, vec!["Error"]);
    assert!(e.raises.is_empty());
    assert_eq!(e.declared, [true, true, false]);
    assert_eq!(e.mask(), 1);
}

#[test]
fn empty_throws_bound_rejects_live_guest_throw_but_not_dead_throw() {
    denied("@Throws()\npub fnc main(): int {throw Error(\"failure\");}", "E_EFFECT_TYPE");
    wasm("@Throws()\npub fnc main(): int {return 42; throw Error(\"dead\");}");
    denied("fnc fails(): int throws(Error) {throw Error(\"failure\");}\n@Throws()\npub fnc main(): int {return fails();}", "E_EFFECT_TYPE");
}

#[test]
fn independent_empty_raise_and_panic_rows_reject_transitive_declared_effects() {
    denied("fnc escalates(): int raises(Error) {return 1;}\n@Raises()\npub fnc main(): int {return escalates();}", "E_EFFECT_TYPE");
    denied("fnc fatal(): int panics(Error) {return 1;}\n@Panics()\npub fnc main(): int {return fatal();}", "E_EFFECT_TYPE");
    wasm("fnc risky(): int throws(Error) {return 42;}\n@Throws(Error)\n@Raises()\n@Panics()\npub fnc main(): int {return risky();}");
}

#[test]
fn duplicate_empty_or_mixed_rows_never_generate_guest_wasm() {
    for src in [
        "@Throws()\n@Throws()\npub fnc main(): int {return 42;}",
        "@Throws()\n@Throws(Error)\npub fnc main(): int {return 42;}",
        "@Throws()\npub fnc main(): int throws(Error) {return 42;}",
        "pub fnc main(): int throws() throws(Error) {return 42;}",
    ] {
        denied(src, "E_EFFECT_TYPE");
    }
}
#[test]
fn invalid_failure_lists_fail_closed() {
    for (src,code) in [
        ("@Throws(Error,)\npub fnc main(): int {return 42;}", "E_EFFECT_TYPE"),
        ("@Throws(Error,Error)\npub fnc main(): int {return 42;}", "E_DUPLICATE"),
        ("@Throws(IOError)\npub fnc main(): int {return 42;}", "E_UNSUPPORTED_TYPE"),
        ("@Throws Error\npub fnc main(): int {return 42;}", "E_SYNTAX"),
        ("pub fnc main(): int Throws() {return 42;}", "E_SYNTAX"),
        ("@Raises() Panics(Error) pub fnc main(): int {return 42;}", "E_TOPLEVEL"),
    ] {
        denied(src,code);
    }
}
