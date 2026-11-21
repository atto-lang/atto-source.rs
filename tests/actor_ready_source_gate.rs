//! Rust compiler gate for a real, BOUNDED terminal READY barrier.
//! This emits a marker into the guest Wasm and the browser host waits for the
//! child worker's actual initialization ack. Arbitrary suspension is still
//! unsupported; no source code executes after a terminal await.
use atto_compiler::{compile, Diagnostic};

fn source(body: &str, ret: &str) -> String {
    format!(
        "define actor Worker as\n  receive(msg: int) -> int {{ return msg + 1; }}\nend\n\
         pub fnc main(): {ret} {{\n  val worker: Worker = spawn Worker();\n  {body}\n}}"
    )
}

fn rejected(src: &str) -> Diagnostic {
    compile(src).expect_err("invalid actor lifecycle source must NOT emit Wasm guest bytes")
}

#[test]
fn terminal_get_ready_emits_distinct_guest_actor_ready_import() {
    let ready = compile(&source("await worker.get_ready();", "void"))
        .expect("terminal initialization await lowers to real guest import");
    assert!(ready.starts_with(b"\0asm"), "expected Wasm module bytes");
    assert!(ready.windows(b"actor_ready".len()).any(|x| x == b"actor_ready"),
        "READY import missing, no fake successful join");
    assert!(!ready.windows(b"actor_join".len()).any(|x| x == b"actor_join"),
        "READY and DONE must remain distinct barriers");

    let done = compile(&source("await worker.get_done();", "void"))
        .expect("existing terminal get_done keeps lowering");
    assert!(done.windows(b"actor_join".len()).any(|x| x == b"actor_join"));
    assert!(!done.windows(b"actor_ready".len()).any(|x| x == b"actor_ready"));
    assert_ne!(ready, done, "READY must not silently lower as DONE");
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
fn arbitrary_nonterminal_ready_continuation_fails_before_guest_emission() {
    let bad = rejected(&source("await worker.get_ready();\n  stdio.println(99);", "void"));
    assert_eq!(bad.code, "E_UNSUPPORTED", "{}", bad.render());
    let bad = rejected(&source("await worker.get_ready();\n  return 42;", "int"));
    assert_eq!(bad.code, "E_UNSUPPORTED", "{}", bad.render());
    let bad = rejected(&source("await worker.get_ready();", "int"));
    assert_eq!(bad.code, "E_UNSUPPORTED", "{}", bad.render());
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
fn rejected_future_compile_does_not_poison_guest_recovery() {
    rejected(&source("await worker.get_ready();\n  stdio.println(9);", "void"));
    let good = compile(&source("return 42;", "int")).expect("valid actor spawn still compiles");
    assert!(good.starts_with(b"\0asm"), "expected real Wasm module bytes");
}
