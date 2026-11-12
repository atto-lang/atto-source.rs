//! Native compiler acceptance for the opt-in task/JSPI guest v2 extension.
//! Wasm execution is separately verified in real browser/JSPI tests.
use atto_compiler::compile;
const TIMER: &str = include_str!("async-fixtures/suspends-await.atto");
const TWO: &str = include_str!("async-fixtures/task-future.atto");
const BARRIER: &str = include_str!("async-fixtures/barrier.atto");
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|part| part == needle)
}
#[test]
fn typed_async_source_emits_real_wasm_and_opt_in_imports() {
    for (source, expected) in [(TIMER,"sleep_ms"),(TWO,"task_start"),(BARRIER,"barrier")] {
        let bytes = compile(source).unwrap_or_else(|e| panic!("{expected}: {e}"));
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
        assert!(contains(&bytes, b"atto.async"), "{expected} missing ABI marker");
        assert!(contains(&bytes, expected.as_bytes()), "{expected} missing import");
        assert_eq!(bytes, compile(source).unwrap(), "nondeterministic lowering");
    }
    let tasks = compile(TWO).unwrap();
    assert!(contains(&tasks, b"task_invoke"), "synthetic Wasm task dispatcher missing");
    assert!(contains(&tasks, b"task_await"), "typed Future await import missing");
}
#[test]
fn async_effect_and_ownership_errors_fail_closed() {
    let failures = [
        ("pub fnc main(): int { return await 42; }", "E_EFFECT"),
        ("pub fnc main(): int { val a = task { return 42; }; return 0; }", "E_EFFECT"),
        ("suspends fnc f(): int { return 42; } pub fnc main(): int { return f(); }", "E_EFFECT"),
        ("pub suspends fnc main(): int { val f = task { return 3; }; return (await f) + (await f); }", "E_MOVED"),
        ("pub suspends fnc main(): int { val x = 20; val f = task { return x; }; return await f; }", "E_UNBOUND"),
        ("pub suspends fnc main(): int { val a = await rt.sleep(true); return a; }", "E_TYPE"),
        ("pub suspends fnc main(): int { return rt.sleep(10); }", "E_EFFECT"),
    ];
    for (source, code) in failures {
        let d = compile(source).expect_err(source);
        assert_eq!(d.code, code, "{source}: {d}");
    }
}
#[test]
fn await_of_ready_scalar_is_valid_and_requires_no_future_or_host_import() {
    let programs = [
        "pub suspends fnc main(): int { await 42; return await 42; }",
        "pub suspends fnc main(): int { val x = await 20; return x + (await 22); }",
        "pub suspends fnc main(): int { val flag = await true; if flag then return 42; fi return 0; }",
    ];
    for source in programs {
        let guest = compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
        assert!(!contains(&guest, b"atto.async"), "immediate await must not claim JSPI suspension");
        assert!(!contains(&guest, b"task_await"), "no Future handle exists for a scalar await");
        assert_eq!(guest, compile(source).unwrap(), "deterministic Wasm lowering");
    }
}
