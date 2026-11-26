use atto_compiler::{compile,compile_verified_strict};
const PRE: &str = "type Bounded = int where self >= 1 and self <= 42; ";
#[test]
fn strict_entry_is_separate_from_ordinary_and_never_falls_back() {
    let plain="pub fnc main() -> int { return 42; }";
    assert!(compile(plain).is_ok());
    assert!(compile_verified_strict(plain).unwrap_err().render().contains("E_VERIFY_UNSUPPORTED"));
    let good=format!("{PRE} pub fnc main() -> int {{ let Bounded x=1; while x<42 {{ x=x+1; }} return x; }}");
    assert!(compile_verified_strict(&good).unwrap().starts_with(b"\0asm"));
    let bad=format!("{PRE} pub fnc main() -> int {{ let int x=1; x=0; return 42/x; }}");
    assert!(compile_verified_strict(&bad).unwrap_err().render().contains("E_VERIFY_UNPROVEN"));
}
