//! Actual native Rust compiler fail-closed regression for unsupported Mutex
//! syntax. This suite executes the real compiler, but intentionally does NOT
//! exercise a compiled .atto guest Mutex runtime; no guest Wasm is emitted.
use atto_compiler::{compile, lexer::lex, mutex_using_preview_gate::reject_unimplemented};

const IMPLICIT: &str = r#"
suspends fnc increment(Mutex<int> counter): int {
  using counter.lock_async() as lock_lease {
    lock_lease.value += 1;
    return lock_lease.value;
  }
}
pub suspends fnc main(): int {
  val counter = new Mutex<int>(0);
  stdio.println(await increment(counter));
  return 0;
}
"#;

const EXPLICIT: &str = r#"
susp fnc increment(Mutex<int> counter): int {
  using await counter.lock_async() as lock_lease {
    lock_lease.value += 1;
    return lock_lease.value;
  }
}
pub susp fnc main(): int {
  val counter = new Mutex<int>(0);
  stdio.println(await increment(counter));
  return 0;
}
"#;

#[test]
fn both_proven_single_task_await_spellings_emit_real_guest() {
    let implicit = compile(IMPLICIT).expect("admitted single-task fast path");
    let explicit = compile(EXPLICIT).expect("same semantics for explicit await");
    assert!(implicit.starts_with(b"\0asm"));
    assert_eq!(implicit, explicit, "contextual await must lower identically");
}

#[test]
fn ready_await_cannot_hide_inside_unlowered_mutex_scope() {
    let source = r#"
pub susp fnc main(): int {
  val counter = new Mutex<int>(0);
  using await counter.lock_async() as lease {
    lease.value += await 42;
  }
  return 0;
}
"#;
    let error = compile(source).expect_err("syntactic await under scoped lease must be rejected");
    assert_eq!(error.code, "E_GUARD_SUSPEND");
}

#[test]
fn constructor_named_mutex_without_scoped_lock_remains_under_existing_type_gate() {
    let src = "pub fnc f(x: Mutex<int>): void { return; }";
    let error = compile(src).expect_err("unlowered Mutex type must fail closed");
    assert_eq!(error.code, "E_UNSUPPORTED_TYPE");
}

#[test]
fn fake_lock_calls_inside_comments_and_strings_do_not_trigger_scope_gate() {
    let text = r#"
// val fake = new Mutex<int>(0); using fake.lock_async() as lease {}
pub fnc main(): int {
  val message = "using mutex.lock_async() Mutex<int>";
  return 17;
}
"#;
    let t = lex(text).expect("lex comments/strings");
    reject_unimplemented(&t).expect("only real scoped Mutex tokens may trigger gate");
    // The ordinary compiler's support for strings is independent of this preflight.
    let guest = compile("pub fnc main(): int { return 17; }")
        .expect("existing scalar guest must remain enabled");
    assert!(guest.starts_with(b"\0asm"));
}

#[test]
fn user_method_name_alone_is_not_a_lock_capability() {
    for source in [
        "pub fnc main(): int { lock_async(); return 0; }",
        "pub fnc main(): int { val x = MyMutex(0); return 0; }",
    ] {
        let t = lex(source).expect("lexer");
        reject_unimplemented(&t).expect("lexical token names cannot manufacture Mutex");
    }
}

#[test]
fn shared_mutex_has_exactly_same_unsupported_scoped_emission_gate() {
    let source = r#"
pub susp fnc main(): int {
  val counter = Sharednew Mutex<int>(0);
  using await counter.lock_async() as lease { lease.value += 1; }
  return 0;
}
"#;
    let error = compile(source).expect_err("no verified TenantVM shared-memory admission");
    assert_eq!(error.code, "E_LOCK_UNSUPPORTED");
}
