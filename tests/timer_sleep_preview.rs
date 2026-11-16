//! Narrow compiler-side JSPI sleep slice on top of the unmerged async runtime.
//! These are compiler tests, NOT real browser timer/cancellation evidence.
use atto_compiler::compile;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|part| part == needle)
}

#[test]
fn duration_sleep_literals_compile_into_versioned_js_pi_timer() {
    for literal in ["0ns", "1ns", "1us", "1ms", "100ms", "1_000ms", "1s", "1000000000ns"] {
        let source = format!("pub suspends fnc main(): int {{ await sleep({literal}); return 42; }}");
        let wasm = compile(&source).unwrap_or_else(|e| panic!("{literal}: {e}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        assert!(contains(&wasm, b"sleep_ms"), "{literal}: timer import missing");
        assert!(contains(&wasm, b"atto.async"), "{literal}: async ABI marker missing");
        assert!(contains(&wasm, b"cooperate"), "{literal}: await yield missing");
        assert_eq!(wasm, compile(&source).unwrap(), "{literal}: nondeterministic compile");
    }
    let qualified = compile("pub suspends fnc main(): int { await std.time.sleep(100ms); return 42; }").unwrap();
    assert!(contains(&qualified, b"sleep_ms"));
    let legacy = compile("pub suspends fnc main(): int { await rt.sleep(25); return 42; }").unwrap();
    assert!(contains(&legacy, b"sleep_ms"), "existing rt.sleep(ms) ABI must remain");
}

#[test]
fn duration_and_blocking_errors_fail_closed_without_guest_wasm() {
    let cases = [
        ("pub fnc main(): int { sleep_blocking(100ms); return 0; }", "E_BLOCKING_CONTEXT"),
        ("pub fnc main(): int { std.time.sleep_blocking(100ms); return 0; }", "E_BLOCKING_CONTEXT"),
        ("pub fnc main(): int { sleep(100ms); return 0; }", "E_EFFECT"),
        ("pub suspends fnc main(): int { sleep(100ms); return 0; }", "E_EFFECT"),
        ("pub suspends fnc main(): int { val x = 1ms; return 0; }", "E_DURATION_TYPE"),
        ("pub suspends fnc main(): int { await sleep(1); return 0; }", "E_DURATION_TYPE"),
        ("pub suspends fnc main(): int { await sleep(1001ms); return 0; }", "E_TIMER_RANGE"),
        ("pub suspends fnc main(): int { await sleep(2s); return 0; }", "E_TIMER_RANGE"),
        ("pub suspends fnc main(): int { await sleep(1.5s); return 0; }", "E_DURATION_LITERAL"),
        ("pub suspends fnc main(): int { await sleep(1e3ms); return 0; }", "E_DURATION_LITERAL"),
        ("pub suspends fnc main(): int { await sleep(1__0ms); return 0; }", "E_DURATION_LITERAL"),
        ("pub suspends fnc main(): int { await sleep(18446744073709551616ns); return 0; }", "E_DURATION_RANGE"),
    ];
    for (source, expected) in cases {
        let d = compile(source).expect_err(source);
        assert_eq!(d.code, expected, "{source}: {d}");
    }
}

#[test]
fn timer_is_not_implicit_integer_or_blocking_api() {
    for source in [
        "pub suspends fnc main(): int { return 100ms; }",
        "pub fnc main(): int { val x = 100ms; return 0; }",
    ] {
        assert_eq!(compile(source).unwrap_err().code, "E_DURATION_TYPE");
    }
}
