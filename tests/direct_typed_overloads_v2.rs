//! Production checker integration for bounded direct typed overloads.
use atto_compiler::compile;
#[test]
fn exact_types_and_arity_compile() {
    for family in [
        "fnc pick(int x): int { return 20; } fnc pick(x: bool): int { return 22; }",
        "fnc pick(x: bool): int { return 22; } fnc pick(int x): int { return 20; }",
    ] {
        let src = format!("{family} fnc pick(int x, int y): int {{ return x+y; }} pub fnc main(): int {{ return pick(1)+pick(true)+pick(0,0); }}");
        assert!(compile(&src).is_ok(), "{:?}", compile(&src));
    }
}
#[test]
fn option_some_and_typed_local_compile() {
    let src = "fnc pick(int x): int { return 20; } fnc pick(x: int?): int { return 22; } pub fnc main(): int { return pick(1)+pick(Some(4)); }";
    assert!(compile(src).is_ok(), "{:?}", compile(src));
    let src = "fnc pick(int? x): int { return 20; } fnc pick(x: Option<Result<int,string>>): int { return 22; } pub fnc main(): int { val v: Option<int> = None; return pick(v); }";
    assert!(compile(src).is_ok(), "{:?}", compile(src));
}
#[test]
fn rejected_calls_and_duplicate_signature() {
    for src in [
        "fnc pick(int? x): int { return 1; } fnc pick(x: Option<Result<int,string>>): int { return 2; } pub fnc main(): int { return pick(None); }",
        "fnc pick(int? x): int { return 1; } fnc pick(x: Option<int>): int { return 2; } pub fnc main(): int { return 42; }",
        "fnc pick(int x): int { return 1; } fnc pick(x: bool): int { return 2; } pub fnc main(): int { return pick(\"wrong\"); }",
        "fnc pick(int x): int { return 1; } fnc pick(x: bool): int { return 2; } pub fnc main(): int { return pick(); }",
    ] {
        assert!(compile(src).is_err(), "unexpected acceptance: {src}");
    }
}
