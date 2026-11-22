//! Executable narrow sync task profile; unsupported concurrency fails closed.
use atto_compiler::{compile, parser::is_reserved};
fn reject(src: &str, code: &str) {
    let d = compile(src).expect_err("unsupported task must not emit guest bytes");
    assert_eq!(d.code, code, "{d}");
}
fn admit(src: &str) {
    let bytes = compile(src).expect("inline task should compile to Wasm");
    assert!(bytes.starts_with(b"\0asm"));
}
#[test]
fn contextual_keywords() {
    assert!(is_reserved("sync"));
    assert!(!is_reserved("synchronous"));
    assert!(!is_reserved("sync_data"));
    assert!(!is_reserved("task"));
    reject("pub fnc main(): int { val sync = 42; return sync; }", "E_RESERVED");
}
#[test]
fn inline_empty_and_scalar_return_profiles() {
    admit("pub fnc main(): int { do sync task {}; return 42; }");
    admit("pub fnc main(): int { do sync task<void> {}; return 42; }");
    admit("pub fnc main(): int { do sync task { return; }; return 42; }");
    admit("pub fnc main(): int { do sync task { defer {}; return; }; return 42; }");
    admit(&format!("pub fnc main(): int {{ do sync task {{ {} return; }}; return 42; }}",
        "defer {};".repeat(32)));
    admit(&format!("pub fnc main(): int {{ let x = 0; do sync task {{ defer {{ {} }}; return; }}; return x; }}",
        "x = 0;".repeat(64)));
    admit("pub fnc main(): int { let x = 0; do sync task { defer { x = 42; }; return; }; return x; }");
    admit("pub fnc main(): int { let x = 0; do sync task<void> { defer { x = 1; }; defer { x = 2; }; return; }; return 41 + x; }");
    admit("pub fnc main(): int { let x = 0; do sync task { defer { x = 42; }; }; return x; }");
    admit("pub fnc main(): int { let x = 0; val v = sync task<int> { defer { x = 2; }; return 40; }; return v + x; }");
    admit("pub fnc main(): int { let x = 0; val v = sync task<int> { defer { x = 2; }; if true { return 40; } else { return 1; } fi }; return v + x; }");
    admit("pub fnc main(): int { do sync task<void> { return; }; return 42; }");
    admit("pub fnc main(): int { do /* gap */ sync /* gap */ task < void > { return; }; return 42; }");
    admit("pub fnc main(): int { block { do sync task {}; } return 42; }");
    admit("pub fnc main(): int { val x = sync task<int> { return 40 + 2; }; return x; }");
    admit("pub fnc main(): int { val x = sync task<int> { val a = 40; let b = a + 1; b = b + 1; return b; }; return x; }");
    admit("pub fnc main(): int { val x = sync task<int> { val a = 40; if a == 40 { return a + 2; } else { return 7; } fi }; return x; }");
    admit("pub fnc main(): int { val x = sync task<int> { block { val y = 42; return y; } }; return x; }");
    admit("pub fnc main(): int { let x = 40; do sync task { x = x + 2; return; }; return x; }");
    admit("pub fnc main(): int { val x = sync task<int> { if true { return 40; } else { return 1; } fi }; return x + 2; }");
    admit("pub fnc main(): int { val x = sync task<int> { if false { return 1; } else { return 40; } fi }; return x + 2; }");
    admit("pub fnc main(): int { let x = 40; do sync task { return; x = 0; }; return x + 2; }");
    admit("pub fnc main(): int { do sync task { val x = 4; if x == 4 { return; } fi }; return 42; }");
    admit("pub fnc main(): int { val x = sync task { return 42; }; return x; }");
    admit("pub fnc main(): int { val x = 40; return sync task<int> { return x + 2; }; }");
    admit("pub fnc main(): int { do sync task {}; val result = sync task<int> { return 40 + 2; }; do sync task {}; return result; }");
    admit("pub fnc main(): int { val x = sync task<bool> { return true; }; return 42; }");
    admit("pub fnc main(): int { val s = sync task<string> { return \"safe\"; }; return 42; }");
    admit("pub fnc main(): int { val x = sync task<int> { if \"ok\" == \"ok\" { return 42; } else { return 0; } fi }; return x; }");
}
#[test]
fn require_do_and_void_only_for_discarded_inline_tasks() {
    // Standalone task syntax cannot silently discard even a void result.
    reject("pub fnc main(): int { sync task {}; return 42; }", "E_SYNTAX");
    reject("pub fnc main(): int { sync task<void> {}; return 42; }", "E_SYNTAX");
    reject("pub fnc main(): int { sync task<int> { return 42; }; return 0; }", "E_SYNTAX");
    reject("pub fnc main(): int { (sync task {}); return 42; }", "E_SYNTAX");
    reject("pub fnc main(): int { ((sync task<void> {})); return 42; }", "E_SYNTAX");
    reject("pub fnc main(): int { (sync task<int> { return 42; }); return 0; }", "E_SYNTAX");
    // The 'do' form cannot have a value-producing type parameter or body,
    // even when the caller would otherwise discard the result.
    reject("pub fnc main(): int { do sync task<int> { return 7; }; return 42; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task<int> {}; return 42; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task<bool> {}; return 42; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task { return 7; }; return 42; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task<void> { return 7; }; return 42; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task<int> { defer { } return 42; }; return 0; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task<bool> { return; }; return 0; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { do sync task { return sync task<int> { return 42; }; }; return 0; }", "E_TASK_RETURN_TYPE");
}

#[test]
fn fail_closed_type_effect_and_cleanup_boundaries() {
    reject("pub fnc main(): int { val x = sync task<int> { return true; }; return 42; }", "E_TYPE");
    reject("pub fnc main(): int { val x = sync task<int> {}; return 42; }", "E_TYPE");
    reject("pub fnc main(): int { val x = sync task<void> {}; return 42; }", "E_TYPE");
    reject("pub fnc main(): int { let x = sync task {}; return 42; }", "E_TYPE");
    reject("pub fnc main(): int { val x = sync task<int> { val y = 41; }; return x; }", "E_RETURN");
    reject("pub fnc main(): int { val x = sync task<int> { if true { return 42; } fi }; return x; }", "E_RETURN");
    reject("pub fnc main(): int { val x = sync task<int> { if true { return 42; } else { return false; } fi }; return x; }", "E_TYPE");
    reject("pub fnc main(): int { do sync task<void> { if true { return 42; } fi }; return 0; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { val x = sync task<int> { val a = 1; block { val b = 2; } return b; }; return x; }", "E_UNBOUND");
    reject("pub fnc main(): int { do sync task { if true { return 7; } else { return; } fi }; return 0; }", "E_TASK_RETURN_TYPE");
    reject("pub fnc main(): int { val x = sync task<int> { return 42; return false; }; return x; }", "E_TYPE");
    reject("pub fnc main(): int { val x = sync task<int> { while true { return 42; } }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { do sync task { val x = stdio.println(42); }; return 42; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val x = sync task<int> { return spawn Worker(); }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val x = sync task<int> { return stdio.println(42); }; return x; }", "E_UNSUPPORTED");
    // The source gate sees string '+' as a scalar Binary. The typed IR
    // must detect its actual bump-heap concatenation and deny emission.
    reject("pub fnc main(): int { val x = sync task<string> { return \"a\" + \"b\"; }; return 42; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val x = sync task<string> { val a = \"a\"; return a + \"b\"; }; return 42; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val x = sync task<int> { if (\"a\" + \"b\") == \"ab\" { return 42; } else { return 0; } fi }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val x = sync task<int> { return await task<int> { return 42; }; }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { x = x + 1; }; }; return x; }", "E_UNSUPPORTED");
    reject(&format!("pub fnc main(): int {{ do sync task {{ {} return; }}; return 42; }}",
        "defer {};".repeat(33)), "E_SOURCE_LIMIT");
    reject(&format!("pub fnc main(): int {{ let x = 0; do sync task {{ defer {{ {} }}; }}; return x; }}",
        "x = 0;".repeat(65)), "E_SOURCE_LIMIT");
    reject("pub fnc main(): int { val x = 0; do sync task { defer { x = 42; }; }; return x; }", "E_IMMUTABLE");
    reject("pub fnc main(): int { do sync task { defer { x = 42; }; let x = 0; }; return 0; }", "E_UNBOUND");
    reject("pub fnc main(): int { let x = 0; do sync task { defer (|int v| -> _ -> { x = v; })(42); }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { x = stdio.println(42); }; }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { x = 1; }; x = x + 1; }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { x = 1; defer { x = 2; }; }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { block { defer { x = 2; }; } }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { val y = 42; }; }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { x = borrow! x; }; }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { x = 2; }; if x == 0 { return; } fi }; return x; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { let x = 0; do sync task { defer { x = 2; }; return sync task<int> { return 42; }; }; return x; }", "E_TASK_RETURN_TYPE");
}
#[test]
fn independent_child_cancellation_not_silently_enabled() {
    reject("pub fnc main(): int { val x = task<int> { return 42; }; return 0; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val x = await task<int> { return 42; }; return 0; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { do task<void> { return; }; return 0; }", "E_UNSUPPORTED");
    // The current browser frontend has no 'susp fnc' declaration grammar;
    // isolate the unsupported task/cancel behavior inside a valid callable.
    reject("pub fnc main(): int { val Future<int> x = task<int> { return 42; }; x.cancel(); return 0; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { val Future<int> worker = task<int> { val lock = new Mutex<int>(41); using await lock.lock_async() as guard { return guard.value; } }; worker.cancel(); return 42; }", "E_UNSUPPORTED");
}
