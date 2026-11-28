use std::path::Path;
#[test]
fn automatic_measurement_prelude_pilot_matches_explicit_catalog() {
    let source="pub fnc main() -> int { val a=4200<Centimeter>.to_unit<Meter>(); val PositiveInt b=a/1<Meter>; return b; }";
    let expected=atto_compiler::compile(&format!("{}\n{source}",include_str!("../stdlib/core/measure.atto"))).unwrap();
    assert_eq!(atto_compiler::compile(source).unwrap(),expected);
    assert_eq!(atto_compiler::compile_with_imports(source,Path::new("main.atto"),|_| panic!("no external loading")).unwrap(),expected);
    assert_eq!(atto_compiler::compile_with_path(source,Path::new("main.atto")).unwrap(),expected);
}
#[test]
fn automatic_measurement_prelude_rejects_bad_units_and_refinements() {
    for (source,code) in [
      ("pub fnc main() -> int { val PositiveInt n=0; return n; }","E_REFINEMENT_BINDING"),
      ("pub fnc main() -> int {\n return 1<Centimeter>.to_unit<Meter>()/1<Meter>; }","E_UNIT_INEXACT"),
      ("pub fnc main() -> int { return 42<Second>; }","E_UNIT_ERASURE"),
    ] {
      let e=atto_compiler::compile(source).unwrap_err();
      assert_eq!(e.code,code);
      assert!(e.pos.line<=source.lines().count() as u32);
    }
    assert!(atto_compiler::compile("pub fnc main() -> int { return 42; }").is_ok());
    assert!(atto_compiler::compile("pub fnc main() -> int { /* 42<Meter> */ return 42; }").is_ok());
}
