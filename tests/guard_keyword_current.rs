use atto_compiler::compile;
#[test]
fn guard_keyword_form_and_return_proof() {
    for source in [
        "pub fnc main():int{guard true else return 1; end return 42;}",
        "pub fnc main():int{guard false else return 42; end return 1;}",
        "pub fnc main():int{if true then guard true {return 1;} else return 2; fi return 42;}",
        "fnc fault(int n):bool{return n+1>0;}pub fnc main():int{guard false,fault(9223372036854775807) else return 42; end return 1;}",
    ] { assert!(compile(source).is_ok(), "{source}"); }
    for source in [
        "pub fnc main():int{guard false else val x=1; end return 42;}",
        "pub fnc main():int{guard false else throw Error(\"bad\"); end return 42;}",
        "pub fnc main():int{guard true,3 else return 1; end return 42;}",
        "pub fnc main():int{guard true {return 1;} else return 2; end return 42;}",
    ] { assert!(compile(source).is_err(), "{source}"); }
}
