//! Constant default operator compiler integration.
use atto_compiler::compile;
#[test]
fn defaults_compile() {
    for formal in ["int? x? ?? 3 ??= 4", "x?: int? ?? 3 ??= 4", "x?: Option<int> ?? Some(3) ??= Some(4)"] {
        let s = format!("fnc f({formal}): int {{ match x over on None -> {{ return 0; }} on Some(n) -> {{ return n; }} end }} pub fnc main(): int {{ return f()+f(None)+f(Some(35)); }}");
        assert!(compile(&s).is_ok(), "{s}: {:?}", compile(&s));
    }
    for s in [
        "fnc f(x?: int ??= 42): int { return x; } pub fnc main(): int { return f(); }",
        "fnc f(int x? ??= 42): int { if x is omitted do return x; fi return 0; } pub fnc main(): int { return f(); }",
        "fnc f(x: int? ?? 42): int { match x over on None -> { return 0; } on Some(n) -> { return n; } end } pub fnc main(): int { return f(None); }",
        "fnc f(x?: int? ?? 42 ??= None): int { match x over on None -> { return 42; } on Some(n) -> { return 0; } end } pub fnc main(): int { return f(); }",
        "fnc f(x?: int? ?? 42): int { match x over on None -> { return 42; } on Some(n) -> { return 0; } end } pub fnc main(): int { return f(); }",
        "fnc f(x?: int ??= -9223372036854775808): int { if x is omitted do return 42; fi return 0; } pub fnc main(): int { return f(); }",
    ] { assert!(compile(s).is_ok(), "{s}: {:?}", compile(s)); }
}
#[test]
fn invalid_defaults_reject() {
    for s in [
        "fnc f(x: int ??= 4): int { return 1; }",
        "fnc f(x?: int ?? 4): int { return 1; }",
        "fnc f(x?: int? ??= 4 ?? 3): int { return 1; }",
        "fnc f(x?: int? ?? 3 ?? 4): int { return 1; }",
        "fnc f(x?: int? ??= 3 ??= 4): int { return 1; }",
        "fnc f(x?: int? ??= \"wrong\"): int { return 1; }",
        "fnc f(x?: int? ??= 1+2): int { return 1; }",
        "fnc f(x?: int? ? ?= 4): int { return 1; }",
        "fnc f(x?: int? ?? = 4): int { return 1; }",
        "fnc f(x?: int? = 4): int { return 1; }",
        "fnc f(x?: int? ??= 9223372036854775808): int { return 1; }",
    ] {
        let source = format!("{s} pub fnc main(): int {{ return 42; }}");
        assert!(compile(&source).is_err(), "unexpected acceptance: {source}");
    }
}
