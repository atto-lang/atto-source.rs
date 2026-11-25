use atto_compiler::compile;
const PRE: &str = "define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000;";
fn src(expr: &str) -> String { format!("{PRE} pub fnc main() -> int {{ return {expr}; }}") }
#[test]
fn spellings_and_checked_traps_are_equivalent() {
    for (compact, alternatives) in [
        ("84<Second> / 2<Second>", ["84::<Second> / 2::<Second>", "(84 as Second) / (2 as Second)"]),
        ("1<Second> / 0<Second>", ["1::<Second> / 0::<Second>", "(1 as Second) / (0 as Second)"]),
        ("-9223372036854775808<Second> / -1<Second>", ["-9223372036854775808::<Second> / -1::<Second>", "(-9223372036854775808 as Second) / (-1 as Second)"]),
    ] {
        let want = compile(&src(compact)).unwrap();
        for expr in alternatives { assert_eq!(compile(&src(expr)).unwrap(), want, "{expr}"); }
    }
}
#[test]
fn scalar_expression_attachment_and_conversion() {
    let want = compile(&src("42")).unwrap();
    for expr in [
        "(40 + 2)::<Second> / 1<Second>",
        "(40 + 2 as Second) / 1<Second>",
        "42000::<Millis>.to_unit<Second>() / 1<Second>",
        "(84<Second> / 2<Second>)::<Second> / 1<Second>",
    ] { assert_eq!(compile(&src(expr)).unwrap(), want, "{expr}"); }
}
#[test]
fn rejects_reinterpretation_and_malformed_attachment() {
    for expr in ["(1000<Millis> as Second) / 1<Second>", "1000<Millis>::<Second> / 1<Second>",
        "1::<Second>::<Second> / 1<Second>", "(1<Second> / 1<Millis>)::<Second> / 1<Second>",
        "(1<Second> as Second) / 1<Second>", "1<Second>::<Millis> / 1<Millis>"] {
        assert_eq!(compile(&src(expr)).unwrap_err().code, "E_UNIT_ATTACHMENT", "{expr}");
    }
    for expr in ["1::<Time>", "1::<Unknown>", "(1 as Time)", "(1 as Unknown)"] {
        assert_eq!(compile(&src(expr)).unwrap_err().code, "E_UNIT_NAME", "{expr}");
    }
    for expr in ["1..<Second>", "1::Second", "1::<Second", "1:<Second>", "1 as Second as Millis"] {
        assert!(compile(&src(expr)).is_err(), "{expr}");
    }
}
#[test]
fn typed_attachment_preserves_private_call_and_local_types() {
    let pre = format!("{PRE} type Duration = int where units in Second;");
    for body in [
        "fnc tag(int n) -> Duration { return n::<Second>; } pub fnc main() -> int { return tag(42) / 1<Second>; }",
        "fnc tag(int n) -> Duration { return n as Second; } pub fnc main() -> int { val Duration a = tag(42); return a / 1<Second>; }",
        "pub fnc main() -> int { val int n = 42; val Duration a = n::<Second>; return a / 1<Second>; }",
    ] { assert!(compile(&format!("{pre} {body}")).is_ok(), "{body}"); }
    let bad = format!("{pre} fnc tag(Duration n) -> Duration {{ return n as Second; }} pub fnc main() -> int {{ return 42; }}");
    assert_eq!(compile(&bad).unwrap_err().code,"E_UNIT_ATTACHMENT");
}
