use atto_compiler::compile;

#[test]
fn guard_dual_forms_and_lazy_comma() {
    for source in [
        "pub fnc main():int{guard true,true else return 1; end return 42;}",
        "pub fnc main():int{guard true,true {return 1;} return 42;}",
        "fnc boom(int x):bool{return x+1>0;} pub fnc main():int{guard false,boom(9223372036854775807) else return 42; end return 1;}",
        "pub fnc main():int{if true then guard true {return 1;} else return 2; fi return 42;}",
    ] { assert!(compile(source).is_ok(), "{source}"); }
}
#[test]
fn guard_fail_closed() {
    for source in [
        "pub fnc main():int{guard false,1 else return 1; end return 42;}",
        "pub fnc main():int{guard false else val int x=1; end return 42;}",
        "pub fnc main():int{guard false {throw Error(\"bad\");} return 42;}",
        "pub fnc main():int{loop {guard false {break; return 1;}} return 42;}",
    ] { assert!(compile(source).is_err(), "{source}"); }
}
