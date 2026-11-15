//! Class-only mtd linkage uses the production checked method path. Actor
//! receivers, namespaces and mutable loans are still rejected without guest
//! artifacts; no caller may infer general actor mtd execution from these tests.
use atto_compiler::compile;

const GOOD: &str = include_str!("fixtures/pass/173-class-mtd-executable.atto");

#[test]
fn concrete_immutable_class_mtd_emits_deterministic_guest_wasm() {
    let wasm = compile(GOOD).unwrap();
    assert_eq!(&wasm[0..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(GOOD).unwrap());
}

#[test]
fn actor_namespace_and_mutable_receivers_remain_unlowered() {
    for (source, code) in [
        ("define class C as end mtd (mut self: C) write() -> int :: 2 end pub fnc main(): int { return 0; }", "E_UNSUPPORTED"),
        ("use namespace x; define class C as end mtd (self: C) test() -> int :: 2 end pub fnc main(): int { return 0; }", "E_UNSUPPORTED"),
        ("declare actor A; mtd (self: A) read() -> int :: 2 end", "E_UNSUPPORTED"),
    ] {
        let d = compile(source).unwrap_err();
        assert_eq!(d.code, code, "{source}: {d}");
    }
}

#[test]
fn signature_duplicates_and_unsupported_types_reject_without_output() {
    for (source, code) in [
        ("define class C as foo(): int { return 1; } end mtd (self: C) foo() -> int :: 2 end", "E_DUPLICATE"),
        ("define class C as end mtd (self: C) weird(List<int> x) -> int :: 2 end", "E_UNSUPPORTED_TYPE"),
        ("define class C as end mtd (self: C) invalid() -> int :: true end pub fnc main(): int { return 0; }", "E_TYPE"),
    ] {
        let d = compile(source).unwrap_err();
        assert_eq!(d.code, code, "{source}: {d}");
    }
}
