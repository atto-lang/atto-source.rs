use atto_compiler::compile;
const PRE: &str = "define dimension Time; define dimension Length; define unit Second: Time; define unit Millis: Time = Second / 1000; define unit Meter: Length; type Duration = int where units in Millis; type Seconds = int where units in Second; type Distance = int where units in Meter;";
fn src(body: &str) -> String { format!("{PRE}\n{body}") }
#[test]
fn admits_typed_local_and_earlier_private_call() {
    for body in [
        "pub fnc main() -> int { return 84 / 2; }",
        "fnc ratio(Duration a, Duration b) -> int { return a / b; } pub fnc main() -> int { val Duration a = 84<Millis>; val b: Duration = 2<Millis>; return ratio(a,b); }",
        "fnc twice(Duration a) -> Duration { return a + a; } pub fnc main() -> int { val Duration a = twice(21<Millis>); return a / 1<Millis>; }",
        "pub fnc main() -> int { val Seconds a = 42000<Millis>.to_unit<Second>(); return a / 1<Second>; }",
        "fnc ratio(Duration a, Duration b) -> int { return a / b; } pub fnc main() -> int { return ratio(1<Millis>,0<Millis>); }",
        "fnc ratio(Duration a, Duration b) -> int { return a / b; } pub fnc main() -> int { return ratio(-9223372036854775808<Millis>,-1<Millis>); }",
    ] {
        let wasm = compile(&src(body)).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
    }
}
#[test]
fn rejects_quantity_boundary_laundering_and_unsupported_forms() {
    for (body,code) in [
        ("pub fnc main() -> int { val Duration a = 1<Second>; return 42; }","E_UNIT_SCALE"),
        ("pub fnc main() -> int { val Duration a = 1<Meter>; return 42; }","E_UNIT_DIMENSION"),
        ("pub fnc main() -> int { val int a = 1<Millis>; return 42; }","E_UNIT_DIMENSION"),
        ("fnc ratio(Duration a) -> int { return a; } pub fnc main() -> int { return 42; }","E_UNIT_DIMENSION"),
        ("fnc ratio(Duration a) -> int { return a / 1<Millis>; } pub fnc main() -> int { return ratio(1<Second>); }","E_UNIT_SCALE"),
        ("pub fnc main() -> int { val Duration a = 1<Millis>; return a; }","E_UNIT_ERASURE"),
        ("pub fnc helper() -> int { return 42; } pub fnc main() -> int { return 42; }","E_REFINEMENT_EXPORT"),
        ("fnc f(Duration a) -> int { return f(a); } pub fnc main() -> int { return 42; }","E_REFINEMENT_CALL"),
        ("pub fnc main() -> int { val Duration a = a; return 42; }","E_UNIT_NAME"),
        ("pub fnc main() -> int { val Duration a = 1<Millis>; val Duration a = 1<Millis>; return 42; }","E_UNIT_DUPLICATE"),
    ] {
        assert_eq!(compile(&src(body)).unwrap_err().code,code,"{body}");
    }
    for body in [
        "pub fnc main() -> int { val Duration a = 1<Millis>; a = 2<Millis>; return 42; }",
        "pub fnc main() -> int { val Duration a = 1<Millis>; return a.to_unit<Second>() / 1<Second>; }",
        "pub fnc main() -> int { return 42; } pub fnc other() -> int { return 42; }",
    ] { assert!(compile(&src(body)).is_err(),"{body}"); }
}

#[test]
fn closed_untyped_quantity_profile_keeps_scalar_division_denial() {
    let source = "define dimension Time; define unit Second: Time; pub fnc main() -> int { return 84 / 2; }";
    assert_eq!(compile(source).unwrap_err().code, "E_UNIT_SCALAR_DIVISION");
}
