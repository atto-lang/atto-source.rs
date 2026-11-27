use atto_compiler::compile;
#[test]
fn string_defaults_and_overloads() {
    for f in ["x?: string? ?= \"bar\" ??= \"star\"", "string? x? ?? \"bar\" ??= \"star\"", "x?: Option<string> ?= Some(\"bar\") ??= Some(\"star\")"] {
        let s = format!("fnc f({f}): int {{ match x over on None -> {{ return 0; }} on Some(v) -> {{ return v.len(); }} end }} pub fnc main(): int {{ return f()+f(None)+f(Some(\"\")); }}");
        assert!(compile(&s).is_ok(), "{s}: {:?}", compile(&s));
    }
    let s = "fnc foo(x?: int? ?= 3 ??= 4): int { return 20; } fnc foo(x?: string? ?= \"bar\" ??= \"star\"): int { return 22; } pub fnc main(): int { return foo(Some(1))+foo(Some(\"x\")); }";
    assert!(compile(s).is_ok(), "{:?}", compile(s));
}
#[test]
fn string_option_ambiguity_and_type_rejection() {
    for a in ["foo()", "foo(None)", "foo(\"x\")"] {
        let s = format!("fnc foo(x?: int? ?= 3 ??= 4): int {{ return 1; }} fnc foo(x?: string? ?= \"bar\" ??= \"star\"): int {{ return 2; }} pub fnc main(): int {{ return {a}; }}");
        assert!(compile(&s).is_err(), "{s}");
    }
    assert!(compile("fnc f(x?: string? ?= 3): int { return 1; } pub fnc main(): int { return 42; }").is_err());
}
