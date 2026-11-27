use atto_compiler::compile_verified_strict;
const PRE:&str="type Index = int where self >= 0 and self <= 1; ";
#[test]
fn fixed_array_cells_have_independent_facts_and_no_heap_escape() {
    for body in [
        "val a=Array.of(1,0); if a[0]!=0 { return 42/a[1]; } return 42;",
        "val a=Array.of(42,9223372036854775807+1); return a[0];",
        "val a=Array.of(42,7); let int i=0; i=2; return a[i];",
    ] {
        let s=format!("{PRE} pub fnc main() -> int {{ {body} }}");
        assert!(compile_verified_strict(&s).unwrap_err().render().contains("E_VERIFY_UNPROVEN"),"{body}");
    }
    for body in ["val a=Array.of(42,7); return a;","val a=Array.of(42,7); val b=a; return b[0];"] {
        let s=format!("{PRE} pub fnc main() -> int {{ {body} }}");
        assert!(compile_verified_strict(&s).unwrap_err().render().contains("E_VERIFY_UNSUPPORTED"));
    }
    let s=format!("{PRE} fnc f(Index i) -> int {{ val a=Array.of(7,42); return a[i]; }} pub fnc main() -> int {{ return f(1); }}");
    assert!(compile_verified_strict(&s).unwrap().starts_with(b"\0asm"));
}
