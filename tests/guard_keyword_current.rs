use atto_compiler::compile;
#[test]
fn guard_keyword_form_and_return_proof() {
    for source in [
        "pub fnc main():int{guard true else return 1; end return 42;}",
        "pub fnc main():int{guard false else return 42; end return 1;}",
        "pub fnc main():int{if true then guard true {return 1;} else return 2; fi return 42;}",
        "fnc fault(int n):bool{return n+1>0;}pub fnc main():int{guard false,fault(9223372036854775807) else return 42; end return 1;}",
    ] { let bytes=compile(source).unwrap_or_else(|e|panic!("{source}: {e}")); assert!(bytes.starts_with(b"\0asm\x01\0\0\0")); assert_eq!(bytes,compile(source).unwrap()); }
    for (source,code) in [
        ("pub fnc main():int{guard false else val x=1; end return 42;}","E_GUARD"),
        ("pub fnc main():int{guard false else throw Error(\"bad\"); end return 42;}","E_GUARD"),
        ("pub fnc main():int{guard true,3 else return 1; end return 42;}","E_TYPE"),
    ] { assert_eq!(compile(source).unwrap_err().code,code,"{source}"); }
    assert!(compile("pub fnc main():int{guard true {return 1;} else return 2; end return 42;}").is_err());
}
