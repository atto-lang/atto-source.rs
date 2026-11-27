use atto_compiler::compile;
#[test]
fn safe_division() {
    for source in [
        "pub fnc main():int{return 39+7/2;}",
        "pub fnc main():int{return 45+(-7/2);}",
        "pub fnc main():int{return 45+(7/-2);}",
        "fnc divide(int n):int{return n/2;} pub fnc main():int{return divide(85);}",
        "pub fnc main():int{guard -9223372036854775808/2 eq -4611686018427387904{return 1;}return 42;}",
        "pub fnc main():int{return -42/-1;}",
    ] { assert!(compile(source).is_ok(), "{source}"); }
}
#[test]
fn unsupported_failing_division() {
    for source in [
        "pub fnc main():int{return 42/0;}",
        "pub fnc main():int{return -9223372036854775808/-1;}",
        "fnc divide(int n,int d):int{return n/d;}pub fnc main():int{return divide(42,1);}",
        "fnc divide(int n):int{return n/-1;}pub fnc main():int{return divide(-42);}",
    ] { assert!(compile(source).is_err(), "{source}"); }
}
