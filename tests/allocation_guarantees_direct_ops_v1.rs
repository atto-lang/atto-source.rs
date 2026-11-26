use atto_compiler::compile;
#[test]
fn scalar_direct_opcode_and_conditional_guarantees() {
    for src in [
        "@noalloc\n@nocopy\nfnc f(x:int):int { return x == 42 ? x : 0; } pub fnc main():int { return f(42); }",
        "@noalloc\nfnc f(x:int):int { return x & 63; } pub fnc main():int { return f(42); }",
        "@nocopy\nfnc f(x:int):int { return x == 42 ? 42 : 0; } pub fnc main():int { return f(42); }",
    ] { assert!(compile(src).is_ok(), "{src}: {:?}", compile(src).err()); }
}
