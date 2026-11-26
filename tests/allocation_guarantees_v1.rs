use atto_compiler::compile;
#[test]
fn canonical_scalar_guarantees() {
    for header in ["define fnc int • forward(int x)", "define fnc int <- forward(int x)", "define fnc forward(int x) -> int", "define fnc forward(int x): int", "define fnc forward(int x): int ->", "define int <- forward(int x)", "int <- forward(int x)"] {
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

#[test]
fn linked_entry_never_discards_guarantees() {
    let src="@noalloc\nfnc f(x:int):int { return x; }\npub fnc main():int { return f(42); }";
    let err=atto_compiler::compile_with_imports(src,std::path::Path::new("main.atto"), |_| panic!("must reject before loading")).unwrap_err();
    assert_eq!(err.code,"E_ANNOTATION_LINK_UNVERIFIED");
}

#[test]
fn abbreviated_headers_have_identical_emission_and_retain_denials() {
    let suffix=" { return data; } pub fnc main():int { return process(42); }";
    for marker in ["", "@nocopy\n", "@noalloc\n", "@noalloc\n@nocopy\n"] {
        let baseline=compile(&format!("{marker}define fnc int <- process(int data){suffix}")).unwrap();
        for header in ["define int <- process(int data)", "int <- process(int data)"] {
            assert_eq!(compile(&format!("{marker}{header}{suffix}")).unwrap(),baseline);
        }
    }
    for src in ["@nocopy\ndefine class X as end", "@nocopy\nlet int x = 42;", "@nocopy\ndefine int <- process(int data):int { return data; }", "@nocopy\nint <- process(int data) { return data+1; }"] {
        assert!(compile(src).is_err(), "{src}");
    }
}
