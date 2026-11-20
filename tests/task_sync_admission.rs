//! Target-only admission: recognize reserved sync and task spellings without
//! faking execution or silently treating a task type parameter as comparison.
use atto_compiler::{compile, parser::is_reserved};

fn rejected_source(source: &str) {
    let d = compile(source).expect_err("task guest lowering must fail closed");
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
}

#[test]
fn sync_reserved_as_whole_word_task_context_only() {
    assert!(is_reserved("sync"));
    assert!(!is_reserved("synchronous"));
    assert!(!is_reserved("sync_data"));
    // 'task' remains contextual during migration.
    assert!(!is_reserved("task"));
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
fn reserved_sync_cannot_be_local_variable() {
    let d = compile("pub fnc main(): int { val sync = 42; return sync; }")
        .expect_err("reserved sync must not become a user variable");
    assert_eq!(d.code, "E_RESERVED", "{d}");
}
