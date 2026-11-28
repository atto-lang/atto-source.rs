//! TaskFuture<T> is a TARGET contract, not a source-compiled runtime in this
//! Rust compiler branch. Never lower an unverified control method into a fake
//! guest call, ignore cancellation, or turn cooperation into a no-op.
//! The full norm lives in atto-spec/task-future-control-v1.md.
use atto_compiler::compile;

#[test]
fn taskfuture_control_forms_are_not_accepted_without_typed_abi() {
    let source_cases = [
        "val TaskFuture<int> t = task<int> { return 42; }; return await t.get_ready();",
        "val TaskFuture<int> t = task<int> { return 42; }; t.cancel_task(); return 42;",
        "val TaskFuture<int> t = task<int> { return 42; }; t.cooperate(); return 42;",
        "val TaskFuture<int> t = task<int> { return 42; }; return await t;",
        "val TaskFuture<int> t = task<int> { return 42; }; val Future<int> r = t.get_ready(); return await r;",
        "val TaskFuture<int> t = task<int> { return 42; }; t.get_ready(); t.get_ready(); return 42;",
        "val TaskFuture<int> t = task<int> { return 42; }; val int a = await t; val int b = await t; return a+b;",
        "val TaskFuture<int> t = task<int> { return 42; }; val Future<int> r = t.get_ready(); t.cancel_task(); return 42;",
        "const TaskFuture<int> t = task<int> { return 42; }; return 42;",
        "val TaskFuture<int> t = sync task<int> { return 42; }; return 42;",
        "val TaskFuture<void> t = task<void> {}; return 42;",
        "do task<void> {}; return 42;",
        "val Future<int> f = task<int> { return 42; }; f.cancel(); return 42;",
        "val Future<int> f = task<int> { return 42; }; f.cooperate(); return 42;",
        "val Future<int> f = task<int> { return 42; }; f.cancel_task(); return 42;",
        "val Future<int> f = task<int> { return 42; }; f.get_ready(); return 42;",
    ];
    for statement in source_cases {
        let source = format!("pub susp fnc main(): int {{ {statement} }}");
        let diagnostic = compile(&source)
            .expect_err("TaskFuture ABI is not implemented: must reject before guest codegen");
        assert!(
            diagnostic.code.starts_with("E_"),
            "a stable guest diagnostic is mandatory: {diagnostic}"
        );
    }
}

#[test]
fn no_taskcontrol_method_is_a_magic_method_on_plain_int() {
    for method in ["get_ready", "cancel_task", "cooperate"] {
        let source = format!("pub fnc main(): int {{ val int v = 42; v.{method}(); return 42; }}");
        compile(&source).expect_err("plain int must never become a task control handle");
    }
}
