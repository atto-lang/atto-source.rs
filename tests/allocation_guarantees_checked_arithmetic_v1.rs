use atto_compiler::compile;

#[test]
fn checked_arithmetic_is_memoryless_but_keeps_overflow_traps() {
    for expr in ["x+1", "x-1", "x*2", "-x", "(x+2)*3"] {
        let body = format!("@noalloc\n@nocopy\nfnc f(x:int):int {{ return {expr}; }} pub fnc main():int {{ return f(42); }}");
        assert!(compile(&body).is_ok(), "{expr}: {:?}", compile(&body).err());
    }
    let chain = std::iter::repeat("1").take(250).collect::<Vec<_>>().join("+");
    let src = format!("@noalloc\n@nocopy\npub fnc main():int {{ return {chain}; }}");
    assert!(compile(&src).is_ok(), "bounded long chain must not overflow proof stack");
}
