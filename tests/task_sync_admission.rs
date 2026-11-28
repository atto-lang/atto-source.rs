//! Target-only admission: recognize reserved sync and task spellings without
//! faking execution or silently treating a task type parameter as comparison.
use atto_compiler::{compile, parser::is_reserved};

fn rejected_source(source: &str) {
    rejected_with(source, "E_UNSUPPORTED");
}

fn rejected_with(source: &str, expected: &str) {
    let d = compile(source).expect_err("task guest lowering must fail closed");
    assert_eq!(d.code, expected, "{d}");
    if expected == "E_UNSUPPORTED_TYPE" {
        assert!(d.message.contains("sealed Future/TaskFuture/ActorFuture awaits"), "{d}");
    }
}

#[test]
fn sync_and_task_reserved_as_whole_words() {
    assert!(is_reserved("sync"));
    assert!(!is_reserved("synchronous"));
    assert!(!is_reserved("sync_data"));
    // Current compiler reserves task; longer identifiers remain ordinary names.
    assert!(is_reserved("task"));
    assert!(!is_reserved("task_data"));
    assert!(compile("pub fnc main(): int { val task_data = 42; return task_data; }").is_ok());
}

#[test]
fn typed_task_and_sync_aliases_never_fall_into_wrong_lowering() {
    rejected_source("pub fnc main(): int { val x = task<int> { return 42; }; return 0; }");
    rejected_source("pub fnc main(): int { val x = task { return 42; }; return 0; }");
    rejected_source("pub fnc main(): int { do task<int> { return 42; }; return 0; }");
    rejected_source("pub fnc main(): int { do sync task<int> { return 42; }; return 0; }");
    rejected_source("pub fnc main(): int { val x = sync task<int> { return 42; }; return 0; }");
    rejected_source("pub fnc main(): int { val x = await task<int> { return 42; }; return 0; }");
}

#[test]
fn inline_sync_not_reinterpreted_as_spawn_or_await_without_verified_lowering() {
    // TARGET: sync task executes in the caller's TaskId, with no independent
    // child admission, Future handle, worker dispatch, or synthetic join.
    // This compiler branch has no authenticated inline MIR or cleanup proof:
    // reject BEFORE producing guest Wasm, even for purely scalar bodies.
    rejected_source(
        "pub fnc main(): int { val int x = sync task<int> { return 42; }; return x; }",
    );
    rejected_source(
        "pub susp fnc main(): int { val int x = sync task<int> { return 42; }; return x; }",
    );
    // TARGET: 'do sync task' must execute inline and discard only the result;
    // never accept it as a silently rewritten 'do task' child spawn.
    rejected_source(
        "pub fnc main(): int { do sync task<int> { return 1; }; return 42; }",
    );
    // A nested independent task is allowed only when the async runtime,
    // suspension effect, capture and physical retirement proofs are live.
    rejected_source(
        "pub susp fnc main(): int { val int x = sync task<int> { return await task<int> { return 42; }; }; return x; }",
    );
}

#[test]
fn sync_inline_lexical_effects_and_order_must_not_be_faked() {
    // Direct parent-local access and source-ordered effects are not evidence
    // of a Future, and cannot be replaced with a schedulable child.
    rejected_source(
        "pub susp fnc main(): int { val int x = sync task<int> { stdio.println(7); return 42; }; stdio.println(9); return x; }",
    );
    // A non-susp enclosing function can only admit transitive non-susp
    // inline effects once the compiler has proven cleanup and no hidden yield.
    rejected_source(
        "pub fnc main(): int { val int x = sync task<int> { return 40 + 2; }; return x; }",
    );
}

#[test]
fn reserved_sync_cannot_be_local_variable() {
    let d = compile("pub fnc main(): int { val sync = 42; return sync; }")
        .expect_err("reserved sync must not become a user variable");
    assert_eq!(d.code, "E_RESERVED", "{d}");
}

#[test]
fn cancellable_typed_task_must_not_emit_guest_until_scope_and_retirement_are_real() {
    // Future.cancel() cannot be treated as a normal method call while the
    // typed task itself lacks cancellation-safe continuation lowering.
    rejected_with(
        "pub susp fnc main(): int {          val Future<int> pending = task<int> { return 42; };          pending.cancel(); return 42; }",
        "E_UNSUPPORTED_TYPE",
    );
    // Discarded tasks must not detach and evade lexical scope cancellation.
    rejected_source(
        "pub susp fnc main(): int {          do task<void> { return; }; return 42; }",
    );
    // Nested cancellation must not be implemented as a normal direct call.
    rejected_with(
        "pub susp fnc main(): int {          val Future<void> pending = task<void> {            do task<void> { return; }; return;          }; pending.cancel(); return 42; }",
        "E_UNSUPPORTED_TYPE",
    );
}

#[test]
fn task_cancellation_lock_releasing_cannot_emit_unproven_guest() {
    // Task <-> Mutex ownership transfer requires runtime-owned lease ledgers,
    // cancellation winner fencing, unwind and physical quiescence evidence.
    // Until those are integrated, reject before producing guest executable bytes.
    rejected_with(
        "pub susp fnc main(): int { \
         val Future<int> worker = task<int> { \
           val lock = new Mutex<int>(41); \
           using await lock.lock_async() as guard { return guard.value; } \
         }; worker.cancel(); return 42; }",
        "E_UNSUPPORTED_TYPE",
    );
    rejected_source(
        "pub susp fnc main(): int { \
         do task<void> { \
           val lock = new Mutex<int>(41); \
           using await lock.lock_async() as guard { guard.value += 1; } \
         }; return 42; }",
    );
}
