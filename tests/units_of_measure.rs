//! Units of measure v1: exact dimension/scale algebra decided at compile time.
use atto_compiler::units::{check_same, conversion_factor, convert_int_exact, Ratio, Registry};

fn reg() -> Registry {
    Registry::default()
}

#[test]
fn seconds_plus_milliseconds_requires_explicit_conversion() {
    let r = reg();
    let e = check_same("+", r.parse("s").unwrap(), r.parse("ms").unwrap()).unwrap_err();
    assert_eq!(e.code, "E_UNIT_SCALE_MISMATCH");
    assert!(check_same("+", r.parse("ms").unwrap(), r.parse("ms").unwrap()).is_ok());
}

#[test]
fn bytes_and_meters_and_seconds_never_mix() {
    let r = reg();
    for (a, b) in [("B", "m"), ("s", "B"), ("KiB", "ms"), ("count", "B")] {
        let e = check_same("+", r.parse(a).unwrap(), r.parse(b).unwrap()).unwrap_err();
        assert_eq!(e.code, "E_UNIT_MISMATCH", "{a} + {b}");
        assert_eq!(
            conversion_factor(r.parse(a).unwrap(), r.parse(b).unwrap()).unwrap_err().code,
            "E_UNIT_MISMATCH"
        );
    }
}

#[test]
fn derived_units_compose_exactly() {
    let r = reg();
    assert_eq!(r.parse("kg*m/s^2").unwrap(), r.parse("N").unwrap());
    assert_eq!(r.parse("N*m").unwrap(), r.parse("J").unwrap());
    assert_eq!(r.parse("J/s").unwrap(), r.parse("W").unwrap());
    assert_eq!(r.parse("1/s").unwrap(), r.parse("Hz").unwrap());
    let speed = r.parse("m").unwrap().div(r.parse("s").unwrap()).unwrap();
    assert_eq!(speed, r.parse("m/s").unwrap());
    assert_eq!(speed.mul(r.parse("s").unwrap()).unwrap(), r.parse("m").unwrap());
}

#[test]
fn conversion_factors_are_exact_rationals() {
    let r = reg();
    assert_eq!(
        conversion_factor(r.parse("KiB").unwrap(), r.parse("B").unwrap()).unwrap(),
        Ratio { num: 1024, den: 1 }
    );
    assert_eq!(
        conversion_factor(r.parse("ms").unwrap(), r.parse("s").unwrap()).unwrap(),
        Ratio { num: 1, den: 1000 }
    );
    assert_eq!(
        conversion_factor(r.parse("h").unwrap(), r.parse("ms").unwrap()).unwrap(),
        Ratio { num: 3_600_000, den: 1 }
    );
    assert_eq!(
        conversion_factor(r.parse("km/h").unwrap(), r.parse("m/s").unwrap()).unwrap(),
        Ratio { num: 5, den: 18 }
    );
    assert_eq!(
        conversion_factor(r.parse("bit").unwrap(), r.parse("B").unwrap()).unwrap(),
        Ratio { num: 1, den: 8 }
    );
    assert_eq!(
        conversion_factor(r.parse("MB").unwrap(), r.parse("MiB").unwrap()).unwrap(),
        Ratio { num: 15625, den: 16384 }
    );
}

#[test]
fn integer_conversion_is_exact_or_rejected() {
    let r = reg();
    assert_eq!(convert_int_exact(3, r.parse("s").unwrap(), r.parse("ms").unwrap()).unwrap(), 3000);
    assert_eq!(convert_int_exact(2, r.parse("KiB").unwrap(), r.parse("B").unwrap()).unwrap(), 2048);
    assert_eq!(
        convert_int_exact(1500, r.parse("ms").unwrap(), r.parse("s").unwrap())
            .unwrap_err()
            .code,
        "E_UNIT_LOSSY"
    );
    assert_eq!(
        convert_int_exact(i64::MAX, r.parse("s").unwrap(), r.parse("ns").unwrap())
            .unwrap_err()
            .code,
        "E_UNIT_OVERFLOW"
    );
}

#[test]
fn dimension_display_is_canonical() {
    let r = reg();
    assert_eq!(r.parse("N").unwrap().dim.to_string(), "m*kg*s^-2");
    assert_eq!(r.parse("B/s").unwrap().dim.to_string(), "s^-1*B");
    assert!(r.parse("m/m").unwrap().dim.is_dimensionless());
}

#[test]
fn malformed_and_unknown_units_fail_closed() {
    let r = reg();
    for (bad, code) in [
        ("", "E_UNIT_SYNTAX"),
        ("m/", "E_UNIT_SYNTAX"),
        ("m^x", "E_UNIT_SYNTAX"),
        ("m^0", "E_UNIT_EXPONENT"),
        ("m^13", "E_UNIT_EXPONENT"),
        ("(m/s)", "E_UNIT_SYNTAX"),
        ("parsec", "E_UNIT_UNKNOWN"),
        ("m^12*m", "E_UNIT_EXPONENT"),
    ] {
        assert_eq!(r.parse(bad).unwrap_err().code, code, "{bad:?}");
    }
}

#[test]
fn exact_downscaling_and_signed_boundary() {
    let r = reg();
    let ms = r.parse("ms").unwrap();
    let s = r.parse("s").unwrap();
    for (input, expected) in [(1000, 1), (-1000, -1), (0, 0), (84000, 84)] {
        assert_eq!(convert_int_exact(input, ms, s).unwrap(), expected);
    }
    let bits = r.parse("bit").unwrap();
    let bytes = r.parse("B").unwrap();
    assert_eq!(convert_int_exact(16, bits, bytes).unwrap(), 2);
    assert_eq!(convert_int_exact(i64::MIN, bits, bytes).unwrap(), i64::MIN / 8);
    assert_eq!(convert_int_exact(-1, bits, bytes).unwrap_err().code, "E_UNIT_LOSSY");
    assert_eq!(convert_int_exact(i64::MIN / 8, bytes, bits).unwrap(), i64::MIN);
    assert_eq!(convert_int_exact(i64::MAX / 8 + 1, bytes, bits).unwrap_err().code, "E_UNIT_OVERFLOW");
}

#[test]
fn raw_invalid_descriptors_fail_without_panics_or_cancellation() {
    use atto_compiler::units::{Dimension, Unit};
    let good = Unit::DIMENSIONLESS;
    for scale in [Ratio { num: 0, den: 0 }, Ratio { num: 1, den: 0 }, Ratio { num: 2, den: 2 }] {
        let bad = Unit { scale, dim: good.dim };
        assert_eq!(bad.mul(good).unwrap_err().code, "E_UNIT_SCALE");
        assert_eq!(bad.div(good).unwrap_err().code, "E_UNIT_SCALE");
        assert_eq!(bad.pow(0).unwrap_err().code, "E_UNIT_SCALE");
        assert_eq!(check_same("+", bad, bad).unwrap_err().code, "E_UNIT_SCALE");
        assert_eq!(conversion_factor(bad, good).unwrap_err().code, "E_UNIT_SCALE");
    }
    let bad = Unit { scale: Ratio::ONE, dim: Dimension([13,0,0,0,0,0,0,0,0]) };
    assert_eq!(bad.div(bad).unwrap_err().code, "E_UNIT_EXPONENT");
    assert_eq!(good.pow(13).unwrap_err().code, "E_UNIT_EXPONENT");
    assert_eq!(good.pow(i8::MIN).unwrap_err().code, "E_UNIT_EXPONENT");
}
