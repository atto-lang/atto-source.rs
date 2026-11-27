use atto_compiler::compile;

#[test]
fn literal_dead_guard_divisions_compile() {
    for source in [
        "pub fnc main():int{let int zero=0;guard false,42/zero eq 0 else return 42; end return 1;}",
        "pub fnc main():int{guard false,42/0 eq 0{return 42;}return 1;}",
        "pub fnc main():int{guard false,-9223372036854775808/-1 eq 0{return 42;}return 1;}",
        "pub fnc main():int{let int zero=0;return true ? 42 : (false ? 0 : 42/zero);}",
    ] {
        let bytes=compile(source).unwrap_or_else(|e|panic!("{source}: {e}"));
        assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(bytes,compile(source).unwrap());
    }
}

#[test]
fn live_division_and_dead_arm_static_errors_remain_denied() {
    for (source,code) in [
        ("pub fnc main():int{let int zero=0;guard true,42/zero eq 0{return 42;}return 1;}","E_UNSUPPORTED"),
        ("pub fnc main():int{let int zero=0;val int n=true ? 42 : 42/zero;return n/zero;}","E_UNSUPPORTED"),
        ("fnc choose(bool b,int d):int{return b ? 42 : 42/d;} pub fnc main():int{return choose(true,0);}","E_UNSUPPORTED"),
        ("pub fnc main():int{return true ? 42 : false/0;}","E_TYPE"),
        ("pub fnc main():int{return true ? 42 : missing/0;}","E_UNBOUND"),
        ("pub fnc main():int{return true ? 42 : (42/0 eq 0);}","E_TYPE"),
    ] {
        assert_eq!(compile(source).unwrap_err().code,code,"{source}");
    }
}
