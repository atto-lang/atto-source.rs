use atto_compiler::{compile, compile_with_imports, compile_verified_strict};
use std::path::Path;

const PRELUDE: &str = "define dimension Time; define unit Second: Time; define unit Millis: Time = Second / 1000;";

#[test]
fn alias_free_quantity_routes_to_checked_wasm_without_loader() {
    let source = format!("{PRELUDE} pub fnc main() -> int {{ return 84<Millis> / 2<Millis>; }}");
    let direct = compile(&source).expect("alias-free checked source");
    assert!(direct.starts_with(b"\0asm"));
    let linked = compile_with_imports(&source, Path::new("main.atto"), |_| {
        panic!("standalone quantity source must not consult import loader");
    }).expect("same typed lane for CLI");
    assert_eq!(direct, linked);
}

#[test]
fn alias_free_quantity_preserves_erasure_and_strict_admission() {
    let source = format!("{PRELUDE} pub fnc main() -> int {{ return 42<Second>; }}");
    assert_eq!(compile(&source).unwrap_err().code, "E_UNIT_ERASURE");
    let source = format!("{PRELUDE} pub fnc main() -> int {{ return 84<Second> / 2<Second>; }}");
    assert_eq!(compile_verified_strict(&source).unwrap_err().code, "E_VERIFY_UNSUPPORTED");
}

#[test]
fn alias_free_quantity_cannot_fall_through_or_publish_partial_source() {
    let source = format!("{PRELUDE} pub fnc main() -> int {{ return 84<Millis> / 2<Millis>; }} pub fnc later() -> int {{ return 1; }}");
    assert!(compile(&source).is_err());
}
