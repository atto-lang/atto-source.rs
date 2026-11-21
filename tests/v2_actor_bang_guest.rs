//! Bang graph admission is strict: only the three-stage bounded Channel<T>
//! checked form is allowed; no direct two-actor trap-path impersonation.
use atto_compiler::{lexer, v2, Diagnostic};

fn compile(source: &str) -> Result<Vec<u8>, Diagnostic> {
    v2::compile(lexer::lex(source)?)
}
const HEADERS: &str = r#"
define actor Producer receives int emits int as
  receive(ActorMail<int> mail): void { emit(mail.value); }
end
define actor Consumer receives int as
  receive(ActorMail<int> mail): void { return; }
end
"#;
fn bad(main: &str, code: &str) {
    let source = format!("{HEADERS}\npub suspends fnc main(): int {{ {main} }}");
    let e=compile(&source).expect_err("unsupported bang graph must never emit guest bytes");
    assert_eq!(e.code,code,"{e}");
}

#[test]
fn checked_bang_and_reverse_spelling_emit_guest_bytes() {
    for (label, src) in [
        ("pipe!", include_str!("fixtures/pass/230-v2-actor-ready-future-pipe.atto")),
        ("rpipe!", include_str!("fixtures/pass/231-v2-await-spawn-ready-pipe.atto")),
    ] {
        let wasm=compile(src).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(wasm.starts_with(b"\0asm"),"{label}: not a guest WebAssembly module");
    }
}

#[test]
fn direct_actor_bang_compiles_as_checked_result_instead_of_legacy_link() {
    for label in [
        include_str!("fixtures/pass/232-v2-direct-actor-pipe-bang.atto"),
        include_str!("fixtures/pass/233-v2-direct-actor-rpipe-bang.atto"),
        include_str!("fixtures/pass/234-v2-direct-actor-bang-duplicate-denied.atto"),
    ] {
        let wasm=compile(label).expect("checked direct actor bang must emit Wasm");
        assert!(wasm.starts_with(b"\0asm"));
    }
}

#[test]
fn non_actor_or_incomplete_checked_graph_must_fail_without_wasm() {
    bad("val p=spawn Producer(); val edge=p pipe! 42; return 0;",
        "E_UNSUPPORTED");
    bad("val p=spawn Producer(); val ch=Channel<int>.bounded(1); val edge=p pipe! ch; return 0;",
        "E_UNSUPPORTED");
}

#[test]
fn mixed_direction_and_mixed_checked_operators_fail_closed() {
    bad("val p=spawn Producer(); val c=spawn Consumer(); val x=p pipe! c rpipe! p; return 0;",
        "E_PIPE_DIRECTION");
    bad("val p=spawn Producer(); val c=spawn Consumer(); val x=p pipe! c pipe p; return 0;",
        "E_PIPE_DIRECTION");
}

#[test]
fn stop_bang_is_typed_actor_graceful_close_not_ambient_kill() {
    bad("stop! 42; return 0;","E_TYPE");
    bad("val p=spawn Producer(); stop!p; return 0;","E_SYNTAX");
}

#[test]
fn malformed_bang_suffix_and_recovery_do_not_publish_guest_graph() {
    bad("val p=spawn Producer(); val c=spawn Consumer(); val edge=p pipe!c; return 0;",
        "E_SYNTAX");
    let good=compile(include_str!("fixtures/pass/230-v2-actor-ready-future-pipe.atto"))
        .expect("valid source must compile after earlier syntax/type errors");
    assert!(good.starts_with(b"\0asm"));
}
