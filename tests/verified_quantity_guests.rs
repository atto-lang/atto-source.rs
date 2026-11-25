use atto_compiler::{compile, compile_with_imports};

const HEADER: &str = "define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000; define unit Tick: Time = Millis;";
fn source(expr: &str) -> String {
    format!("{HEADER} pub fnc main() -> int {{ return {expr}; }}")
}

#[test]
fn exact_closed_quantities_use_the_normal_wasm_emitter() {
    let scalar = compile("pub fnc main() -> int { return 42; }").unwrap();
    for expr in [
        "84000<Millis>.to_unit<Second>() / 2<Second>",
        "84<Tick> / 2<Millis>",
        "0 - (-84<Millis> / 2<Millis>)",
        "42 + (0<Millis>.to_unit<Second>() / 1<Second>)",
    ] {
        assert_eq!(compile(&source(expr)).unwrap(), scalar, "{expr}");
    }
}

#[test]
fn mismatch_inexact_scale_and_erasure_fail_closed() {
    for (expr, expected) in [
        ("1<Second> + 1<Millis>", "E_UNIT_SCALE"),
        ("1500<Millis>.to_unit<Second>() / 1<Second>", "E_UNIT_INEXACT"),
        ("1<Second> / 1<Millis>", "E_UNIT_ERASURE"),
        ("1<Missing>", "E_UNIT_NAME"),
        ("1 <Second>", "E_SYNTAX"),
        ("9223372036854775807<Second>.to_unit<Millis>()", "E_UNIT_RANGE"),
        ("1<Second> / 0<Second>", "E_UNSUPPORTED"),
    ] {
        assert_eq!(compile(&source(expr)).unwrap_err().code, expected, "{expr}");
    }
}

#[test]
fn declaration_identity_and_cycles_fail_closed() {
    for (decl, expected) in [
        ("define dimension Time; define dimension Time;", "E_UNIT_DUPLICATE"),
        ("define dimension Time; define unit A: Time = B; define unit B: Time = A;", "E_UNIT_NAME"),
        ("define dimension Time; define dimension Size; define unit A: Time; define unit B: Size = A;", "E_UNIT_DIMENSION"),
        ("define dimension Time; define unit A: Time; define unit B: Time;", "E_UNIT_DUPLICATE"),
    ] {
        let src = format!("{decl} pub fnc main() -> int {{ return 42; }}");
        assert_eq!(compile(&src).unwrap_err().code, expected);
    }
    assert!(compile("pub fnc main() -> int { int unit = 40; int dimension = 2; return unit + dimension; }").is_ok());
}

#[test]
fn standalone_cli_path_does_not_load_imports_or_admit_trailing_code() {
    let path = std::path::Path::new("main.atto");
    let good = source("84<Millis> / 2<Millis>");
    assert!(compile_with_imports(&good, path, |_| panic!("must not load")).is_ok());
    let bad = format!("{good} pub fnc other() -> int {{ return 7; }}");
    assert!(compile(&bad).is_err());
    let imported = format!("import core.num; {good}");
    assert!(compile_with_imports(&imported, path, |_| panic!("must not load")).is_err());
}

#[test]
fn nominal_scales_cross_128_bits_and_stop_at_256() {
    let mut header = String::from("define dimension Time; define unit Base: Time;");
    let factor = 1u64 << 62;
    let mut prior = String::from("Base");
    for n in 1..=4 {
        let next = format!("U{n}");
        header.push_str(&format!(" define unit {next}: Time = {prior} * {factor};"));
        prior = next;
    }
    header.push_str(" define unit MaxScale: Time = U4 * 128;");
    let make = |h: &str, e: &str| format!("{h} pub fnc main() -> int {{ return {e}; }}");
    let scalar = compile("pub fnc main() -> int { return 42; }").unwrap();
    assert_eq!(compile(&make(&header, "84<MaxScale> / 2<MaxScale>")).unwrap(), scalar);
    assert_eq!(compile(&make(&header, "42 + 0<MaxScale>.to_unit<Base>() / 1<Base>")).unwrap(), scalar);
    let extra = format!("{header} define unit TooWide: Time = MaxScale * 2;");
    assert_eq!(compile(&make(&extra, "42")).unwrap_err().code, "E_UNIT_LIMIT");
}

#[test]
fn generated_exact_conversions_and_inexact_neighbors() {
    let scalar = compile("pub fnc main() -> int { return 42; }").unwrap();
    for factor in [2i64,3,5,7,8,10,100,1000,1000000] {
        let h = format!("define dimension Time; define unit Base: Time; define unit Small: Time = Base / {factor};");
        for expr in [
            format!("{}<Small>.to_unit<Base>() / 1<Base>", 42*factor),
            format!("0 - (-{}<Small>.to_unit<Base>() / 1<Base>)", 42*factor),
            format!("42<Base>.to_unit<Small>() / {factor}<Small>"),
        ] {
            let src = format!("{h} pub fnc main() -> int {{ return {expr}; }}");
            assert_eq!(compile(&src).unwrap(), scalar, "{expr}");
        }
        let bad = format!("{h} pub fnc main() -> int {{ return {}<Small>.to_unit<Base>() / 1<Base>; }}", 42*factor+1);
        assert_eq!(compile(&bad).unwrap_err().code, "E_UNIT_INEXACT");
    }
}

#[test]
fn malformed_literal_and_suffix_controls() {
    for expr in ["0b101010", "0b101010<Millis>", "84 /* gap */ <Millis> / 2<Millis>",
        "(84<Millis>).to_unit<Second>()", "84<Millis>.to_unit<Second>(1)",
        "84<Millis>.to_unit<Second>().to_unit<Millis>()", "84<Millis> / 2<Millis> trailing"] {
        assert!(compile(&source(expr)).is_err(), "{expr}");
    }
    assert!(compile("define dimension T; define unit A: T; define unit B: T = A * 0b10; pub fnc main() -> int { return 42; }").is_err());
    assert!(compile("pub fnc main() -> int { int protocol = 42; return protocol; }").is_ok());
}
