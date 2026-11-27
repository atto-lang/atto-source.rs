//! Bounded direct Option omission and provenance; explicit defaults gated.
use atto_compiler::compile;
#[test]
fn direct_omission_and_provenance_compile() {
    for formal in ["int? x?", "x?: int?", "x?: Option<int>"] {
        let s = format!("fnc probe({formal}): int {{ if x is omitted do return 20; fi return 1; }} pub fnc main(): int {{ return probe()+probe(None)+probe(Some(4)); }}");
        assert!(compile(&s).is_ok(), "{:?}", compile(&s));
    }
}
#[test]
fn type_and_arity_resolution_compile() {
    for s in [
        "fnc f(int x): int { return 20; } fnc f(int x, y?: int?): int { return 22; } pub fnc main(): int { return f(1)+f(1,None); }",
        "fnc f(x?: int?, y: int): int { if x is omitted do return y; fi return 0; } pub fnc main(): int { return f(omit,42); }",
        "fnc f(x?: int?): int { if x is omitted do return 42; fi return 0; } pub fnc main(): int { return f(_); }",
        "fnc f(x?: int?): int { return 20; } fnc f(x?: Option<Result<int,string>>): int { return 22; } pub fnc main(): int { val v: Option<int> = None; return f(v); }",
    ] {
        assert!(compile(s).is_ok(), "{s}: {:?}", compile(s));
    }
}
#[test]
fn invalid_provenance_and_omission_reject() {
    for s in [
        "fnc f(x: int?): bool { return x is omitted; } pub fnc main(): int { return 42; }",
        "fnc f(x?: int?): bool { val y: int = 1; return y is omitted; } pub fnc main(): int { return 42; }",
        "fnc f(x?: int?): bool { if true do val x: int = 1; return x is omitted; fi return false; } pub fnc main(): int { return 42; }",
        "fnc f(x?: int): int { return 1; } pub fnc main(): int { return 42; }",
        "fnc f(x?: int?, y: int): int { return y; } pub fnc main(): int { return f(Some(1)); }",
        "fnc f(x?: int?): int { return 1; } fnc f(x?: Option<Result<int,string>>): int { return 2; } pub fnc main(): int { return f(); }",
        "fnc f(x?: int?): int { return 1; } fnc f(x?: Option<Result<int,string>>): int { return 2; } pub fnc main(): int { return f(None); }",
        "fnc f(x?: int?): int { return 1; } fnc f(x: Option<int>): int { return 2; } pub fnc main(): int { return 42; }",
    ] {
        assert!(compile(s).is_err(), "unexpected acceptance: {s}");
    }
}
