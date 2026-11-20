// Compiles REAL .atto source with the provisional browser v2 compiler;
// it does not substitute host-side Rust message routing. Browser e2e tests
// separately execute the emitted guest Wasm in a Chromium worker.
use atto_compiler::{lexer, v2};

fn compile_v2(source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    v2::compile(lexer::lex(source)?)
}

#[test]
fn legacy_actor_pipe_and_rpipe_compile_to_guest_wasm_not_host_simulation() {
    for (name, source) in [
        ("pipe", include_str!("fixtures/pass/207-v2-actor-pipe-forward.atto")),
        ("rpipe", include_str!("fixtures/pass/208-v2-actor-rpipe-reverse.atto")),
    ] {
        let guest = compile_v2(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(guest.starts_with(b"\0asm"), "{name}: real Wasm guest magic");
        assert!(guest.len() > 8, "{name}: nonempty guest module");
    }
}

#[test]
fn reverse_pipe_mixes_and_untyped_sources_fail_before_wasm_emission() {
    for (name, source, code) in [
        ("mixed", include_str!("fixtures/fail/208-v2-actor-rpipe-direction-mix.atto"),
            "E_PIPE_DIRECTION"),
        ("no emit", include_str!("fixtures/fail/209-v2-actor-pipe-missing-source.atto"),
            "E_TYPE"),
        ("type mismatch", include_str!("fixtures/fail/210-v2-actor-rpipe-type-mismatch.atto"),
            "E_TYPE"),
    ] {
        let e = compile_v2(source).expect_err("invalid continuous graph must not emit Wasm");
        assert_eq!(e.code, code, "{name}: {e}");
    }
}

#[test]
fn functional_pipe_is_not_continuous_actor_pipe() {
    let source = r#"
fnc double(int x): int { return x * 2; }
pub suspends fnc main(): int { return 21 |> double; }
"#;
    let wasm = compile_v2(source).expect("functional value application remains distinct");
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn actor_runtime_rejects_wrong_direction_even_if_actor_types_are_nominally_valid() {
    let source = r#"
define actor Producer as
  receive(ActorMail<int> mail): void { emit(mail.value); return; }
end
define actor Consumer as
  receive(ActorMail<int> mail): void { return; }
end
pub suspends fnc main(): int {
  val producer = spawn Producer();
  val consumer = spawn Consumer();
  val backwards = consumer pipe producer;
  return 0;
}
"#;
    let e = compile_v2(source).expect_err("sink-only Consumer is not a DATA source");
    assert_eq!(e.code, "E_TYPE");
}
