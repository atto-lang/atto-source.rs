//! The existing browser trap must discharge only ordinary throw, never
//! declared escalation or fatal panic. These are effect admission tests;
//! #275 later replaces the legacy Option trap payload with Result.
use atto_compiler::compile;

#[test]
fn trapped_throw_is_compatible_with_explicit_empty_throws_bound() {
    let src = include_str!("fixtures/pass/230-effect-empty-throws-trap.atto");
    let bytes = compile(src).expect("ordinary throw is discharged by caller trap");
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn trap_never_hides_raise_or_panic_effects() {
    for effect in ["raises", "panics"] {
        let src = format!("fnc risky(): int {effect}(Error) {{return 42;}}\n@Throws()\npub fnc main(): int {{val Option<int> x = trap risky(); return 42;}}");
        let err = compile(&src).expect_err("trap is not permitted to discharge raise or panic");
        assert_eq!(err.code, "E_EFFECT_TYPE", "{src}: {err}");
    }
}

#[test]
fn unhandled_guest_throw_still_violates_explicit_empty_bound() {
    let src = "fnc risky(): int throws(Error) {throw Error(\"failure\");}\n@Throws()\npub fnc main(): int {return risky();}";
    let err = compile(src).expect_err("untrapped throw must be rejected");
    assert_eq!(err.code, "E_EFFECT_TYPE");
}
