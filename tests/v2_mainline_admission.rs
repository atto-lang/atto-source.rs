//! Mainline integration admission: explicit v2 only, no default fallback.
//! These are compiler-level tests; guest execution is a separate Node suite.
use atto_compiler::{compile, compile_experimental_v2, Diagnostic};

const READY: &str = include_str!("fixtures/pass/230-v2-actor-ready-future-pipe.atto");
const DIRECT: &str = include_str!("fixtures/pass/232-v2-direct-actor-pipe-bang.atto");

fn is_guest_wasm(bytes: &[u8]) -> bool {
    bytes.len() > 8 && bytes.starts_with(b"\0asm")
}

#[test]
fn mainline_experimental_profile_compiles_real_ready_and_checked_bang() {
    for source in [
        READY,
        include_str!("fixtures/pass/231-v2-await-spawn-ready-pipe.atto"),
        DIRECT,
        include_str!("fixtures/pass/233-v2-direct-actor-rpipe-bang.atto"),
        include_str!("fixtures/pass/234-v2-direct-actor-bang-duplicate-denied.atto"),
    ] {
        let wasm=compile_experimental_v2(source).expect("explicit v2 guest compiles");
        assert!(is_guest_wasm(&wasm),"no synthetic source-only success");
    }
}

#[test]
fn mainline_default_compiler_must_not_fallback_to_experimental_actor_runtime() {
    for source in [READY,DIRECT] {
        let error=compile(source).expect_err("no implicit async guest fallback");
        assert!(!error.code.is_empty());
    }
}

#[test]
fn missing_explicit_effect_and_invalid_actor_lifecycle_are_fail_closed() {
    let invalid=r#"
define actor Worker receives int as
  receive(ActorMail<int> mail): void { return; }
end
pub fnc main(): int {
  val w=spawn Worker();
  await w.get_ready();
  return 42;
}
"#;
    let err=compile_experimental_v2(invalid)
        .expect_err("await requires an explicitly suspending callable");
    assert_eq!(err.code,"E_AWAIT");
}

#[test]
fn source_cap_is_enforced_before_lexing_or_guest_emission() {
    let too_large="a".repeat(atto_compiler::SOURCE_CAP+1);
    let err=compile_experimental_v2(&too_large)
        .expect_err("source-size gate must reject without bytes");
    assert_eq!(err.code,"E_SOURCE_LIMIT");
}

#[test]
fn removed_boolean_operators_are_not_restored_in_experimental_v2() {
    for op in ["^^","&&","||"] {
        let source=format!("pub suspends fnc main(): int {{ return 1 {op} 2; }}");
        let result: Result<Vec<u8>,Diagnostic>=compile_experimental_v2(&source);
        assert!(result.is_err(),"removed {op} operator may not emit a guest");
    }
}
