//! Guest compiler contract: recognize the canonical ActorRef READY API without
//! claiming wasm32 source Future/continuation lowering or emitting a fake guest.
use atto_compiler::{compile, Diagnostic};

fn source(body: &str, ret: &str) -> String {
    format!(
        "define actor Worker as\n  receive(msg: int) -> int {{ return msg + 1; }}\nend\n\
         pub fnc main(): {ret} {{\n  val worker: Worker = spawn Worker();\n  {body}\n}}"
    )
}

fn rejected(src: &str) -> Diagnostic {
    compile(src).expect_err("unsupported actor lifecycle source must NOT emit Wasm guest bytes")
}

#[test]
fn terminal_get_ready_is_recognized_but_not_falsely_lowered_as_get_done() {
    let bad = rejected(&source("await worker.get_ready();", "void"));
    assert_eq!(bad.code, "E_UNSUPPORTED");
    assert!(bad.message.contains("get_ready()"), "{}", bad.render());
    assert!(bad.message.contains("Future"), "{}", bad.render());
    assert!(bad.pos.line > 0 && bad.pos.col > 0);
}

#[test]
fn get_ready_is_zero_argument_and_requires_a_real_actor_ref() {
    let bad = rejected(&source("await worker.get_ready(42);", "void"));
    assert_eq!(bad.code, "E_ARITY", "{}", bad.render());
    assert!(bad.message.contains("get_ready()"));
    let nonactor = rejected(
        "pub fnc main(): void { val number: int = 42; await number.get_ready(); }"
    );
    assert!(["E_TYPE", "E_UNSUPPORTED"].contains(&nonactor.code));
}

#[test]
fn method_value_is_a_future_not_a_scalar_or_implicit_boolean() {
    let bad = rejected(&source("val ready = worker.get_ready();\n  return 0;", "int"));
    assert_eq!(bad.code, "E_UNSUPPORTED", "{}", bad.render());
    assert!(bad.message.contains("get_ready()"));
    let bad_ready_property = rejected(&source("val status = worker.ready;\n  return 0;", "int"));
    assert_ne!(bad_ready_property.code, "");
    let bad_done_property = rejected(&source("val status = worker.done;\n  return 0;", "int"));
    assert_ne!(bad_done_property.code, "");
}

#[test]
fn failed_ready_compile_does_not_poison_compiler_state() {
    rejected(&source("await worker.get_ready();", "void"));
    let good = compile(&source("return 42;", "int")).expect("valid actor spawn still compiles");
    assert!(good.starts_with(b"\0asm"), "expected real Wasm module bytes");
}
