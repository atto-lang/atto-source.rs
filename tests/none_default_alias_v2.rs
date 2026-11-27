//! The ?= formal alias has no independent semantics or writeback.
use atto_compiler::compile;
#[test]
fn alias_and_legacy_spellings_compile() {
    for formal in ["int? x? ?= 3 ??= 4", "x?: int? ?= 3 ??= 4", "x?: Option<int> ?= Some(3) ??= Some(4)", "int? x ?= 3", "x: int? ?= 3", "x: Option<int> ?= 3", "x?: int? ?? 3 ??= 4"] {
        let s = format!("fnc f({formal}): int {{ match x over on None -> {{ return 0; }} on Some(n) -> {{ return n; }} end }} pub fnc main(): int {{ return f(None); }}");
        assert!(compile(&s).is_ok(), "{s}: {:?}", compile(&s));
    }
}
#[test]
fn postfix_and_ternary_stay_distinct() {
    let s = "fnc f(x?: int? ?= 3 ??= 4): Option<int> { val n: int = x?; return Some(n+1); } pub fnc main(): int { val v: Option<int> = f(None); match v over on None -> { return 0; } on Some(n) -> { return n; } end }";
    assert!(compile(s).is_ok(), "{:?}", compile(s));
    let s = "fnc f(x?: int? ?= 3 ??= 4): int { return x is omitted ? 42 : 0; } pub fnc main(): int { return f(); }";
    assert!(compile(s).is_ok(), "{:?}", compile(s));
}
#[test]
fn reject_duplicate_and_split_aliases() {
    for formal in ["x?: int? ?= 1 ?? 2", "x?: int? ?? 1 ?= 2", "x?: int? ?= 1 ?= 2", "x?: int? ??= 1 ?= 2", "x?: int ?= 1", "x?: int? ? = 1", "x?: int? ?/*split*/= 1", "x?: int? ?= 1 ?? = 2"] {
        let s = format!("fnc f({formal}): int {{ return 1; }} pub fnc main(): int {{ return 42; }}");
        assert!(compile(&s).is_err(), "unexpected acceptance: {s}");
    }
    assert!(compile("pub fnc main(): int { val x: Option<int> = None; x ?= 42; return 42; }").is_err());
}
