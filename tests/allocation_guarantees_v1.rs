use atto_compiler::compile;
#[test]
fn canonical_scalar_guarantees() {
    for header in ["define fnc int • forward(int x)", "define fnc int <- forward(int x)", "define fnc forward(int x) -> int", "define fnc forward(int x): int", "define fnc forward(int x): int ->"] {
        let src=format!("@noalloc\n@nocopy\n{header} {{ return x; }}\npub fnc main(): int {{ return forward(42); }}");
        assert!(compile(&src).is_ok(), "{header}: {:?}", compile(&src).err());
    }
}
#[test]
fn transitive_and_fail_closed() {
    let src="@noalloc\n@nocopy\nfnc forward(x:int):int { return leaf(x); }\nfnc leaf(x:int):int { return x; }\npub fnc main():int { return forward(42); }";
    assert!(compile(src).is_ok());
    for (src, code) in [
        ("@noalloc\nfnc f(x:int):int { return f(x); } pub fnc main():int { return f(42); }","E_NOALLOC"),
        ("@nocopy\nfnc f(x:int):int { return x+1; } pub fnc main():int { return f(42); }","E_NOCOPY"),
        ("@noalloc\n@noalloc\npub fnc main():int { return 42; }","E_ANNOTATION"),
        ("@noalloc(1)\npub fnc main():int { return 42; }","E_ANNOTATION"),
        ("pub fnc main():int { @nocopy return 42; }","E_ANNOTATION"),
    ] { assert_eq!(compile(src).unwrap_err().code,code, "{src}"); }
}
