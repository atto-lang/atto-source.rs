//! Admission tests for the real source compiler, not implemented spread parity.
//! Valid source spreads currently fail closed before emitting either target.
use atto_compiler::{compile, compile_js};

#[test]
fn adjacent_ellipsis_is_unsupported_not_malformed_syntax() {
    let sources = [
        "fnc f(int a): int { return a; } pub fnc main(): int { return f(...(42,)); }",
        "fnc f(): int { return 42; } pub fnc main(): int { return f(...()); }",
        "fnc f(int a): int { return a; } pub fnc main(): int { return f(...{a:42}); }",
    ];
    for source in sources {
        let wasm = compile(source).expect_err("spread not lowered");
        let js = compile_js(source).expect_err("spread not lowered");
        assert_eq!(wasm.code, "E_UNSUPPORTED", "{source}: {wasm}");
        assert_eq!(wasm.message, js.message, "{source}: {js}");
        assert_eq!(wasm.code, js.code);
        assert!(wasm.message.contains("spread arguments"), "{wasm}");
    }
}

#[test]
fn single_dot_and_separated_dots_are_not_spread_tokens() {
    for spelling in [".42", ". . .42"] {
        let source = format!("fnc f(int a): int {{ return a; }} pub fnc main(): int {{ return f({spelling}); }}");
        let err = compile(&source).expect_err("malformed argument");
        assert!(!err.message.contains("spread arguments"), "{err}");
    }
}

#[test]
fn ordinary_zero_and_one_argument_calls_remain_admitted() {
    for source in [
        "fnc f(): int { return 42; } pub fnc main(): int { return f(); }",
        "fnc f(int a): int { return a; } pub fnc main(): int { return f(42); }",
    ] {
        assert!(!compile(source).unwrap().is_empty());
        assert!(!compile_js(source).unwrap().is_empty());
    }
}
