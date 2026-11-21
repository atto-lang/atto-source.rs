//! Compiler evidence for READY/DONE as distinct Futures in the *real* v2
//! guest lowering; native/Wasm execution lives in node-v2-pipe-guest.mjs.
//! This experimental guest-memory profile does not claim TenantVM isolation.
use atto_compiler::{lexer, v2, Diagnostic};

fn compile_v2(source: &str) -> Result<Vec<u8>, Diagnostic> {
    v2::compile(lexer::lex(source)?)
}

#[test]
fn ready_future_and_await_spawn_emit_real_wasm_guest() {
    for (label, source) in [
        ("ready future", include_str!("fixtures/pass/230-v2-actor-ready-future-pipe.atto")),
        ("await spawn", include_str!("fixtures/pass/231-v2-await-spawn-ready-pipe.atto")),
    ] {
        let bytes=compile_v2(source).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(bytes.starts_with(b"\0asm"), "{label}: missing Wasm magic");
        assert!(bytes.len()>8, "{label}: guest must contain more than header");
    }
}

const ACTOR: &str = r#"
define actor Worker receives int as
  receive(ActorMail<int> mail): void { return; }
end
"#;

fn invalid(main: &str, expected: &str) {
    let source=format!("{ACTOR}\n{main}");
    let problem=compile_v2(&source).expect_err("unsupported actor form must emit NO guest");
    assert_eq!(problem.code,expected,"{problem}");
}

#[test]
fn readiness_is_zero_arg_method_and_properties_are_not_aliases() {
    invalid(
        "pub suspends fnc main(): int { val w=spawn Worker(); await w.get_ready(42); return 0; }",
        "E_ARITY");
    invalid(
        "pub suspends fnc main(): int { val w=spawn Worker(); val r=w.ready; return 0; }",
        "E_TYPE");
    invalid(
        "pub suspends fnc main(): int { val w=spawn Worker(); val r=w.done; return 0; }",
        "E_TYPE");
}

#[test]
fn await_spawn_requires_suspends_and_actor_type() {
    invalid("pub fnc main(): int { val w=await spawn Worker(); return 0; }","E_AWAIT");
    invalid("pub suspends fnc main(): int { val w=await spawn Missing(); return 0; }","E_UNDEFINED");
}

#[test]
fn unsupported_source_does_not_poison_next_valid_guest() {
    invalid("pub suspends fnc main(): int { val w=spawn Worker(); await w.get_ready(1); return 0; }","E_ARITY");
    let good=compile_v2(include_str!("fixtures/pass/231-v2-await-spawn-ready-pipe.atto"))
        .expect("valid future guest after rejection");
    assert!(good.starts_with(b"\0asm"));
}
