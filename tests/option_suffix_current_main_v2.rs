//! Production parser/compiler regression tests; bounded Option ABI only.
use atto_compiler::{compile, lexer::lex, parser::Parser, ast::Type};

#[test]
fn required_formals_normalize_both_orders() {
    for source in [
        "fnc f(int? x): int { return 42; }",
        "fnc f(x: int?): int { return 42; }",
        "fnc f(Option<int> x): int { return 42; }",
    ] {
        let p = Parser::new(lex(source).unwrap()).parse_program().unwrap();
        assert_eq!(p.funcs[0].params[0].ty, Type::OptionInt);
    }
}
#[test]
fn supported_option_guest_compiles() {
    for formal in ["int? x", "x: int?", "Option<int> x"] {
        let source = format!("fnc f({formal}): int {{ match x over on None -> {{ return 42; }} on Some(n) -> {{ return n; }} end }} pub fnc main(): int {{ return f(None); }}");
        assert!(compile(&source).is_ok(), "{formal}: {:?}", compile(&source));
    }
}
#[test]
fn unsupported_scalar_omission_and_nested_suffix_stay_closed() {
    for source in [
        "fnc f(int x?): int { return 42; } pub fnc main(): int { return f(); }",
        "fnc f(x?: int): int { return 42; } pub fnc main(): int { return f(); }",
        "fnc f(int?? x): int { return 42; } pub fnc main(): int { return 42; }",
        "fnc f(string? x): int { return 42; } pub fnc main(): int { return 42; }",
        "fnc f(int? x): int { return 42; } pub fnc main(): int { return f(); }",
        "fnc f(int? x): int { return 42; } pub fnc main(): int { return f(42); }",
    ] {
        assert!(compile(source).is_err(), "unexpected acceptance: {source}");
    }
}
