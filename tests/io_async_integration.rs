//! Integration boundary between JSPI Future/await (#77) and typed I/O
//! direction and detached syntax (#176). The ordinary stdout profile is
//! executable; nb/do channel I/O MUST fail closed until runtime support.
use atto_compiler::compile;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|segment| segment == needle)
}

#[test]
fn captured_stdout_and_future_await_compile_together_without_conflating_effects() {
    let src = r#"
pub suspends fnc main(): int {
    val task_result = task { return 42; };
    stdio.out << "registered";
    return await task_result;
}
"#;
    let wasm=compile(src).expect("existing direct stdout and task await compile together");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert!(contains(&wasm,b"atto.async"),"Future task requires versioned async marker");
    assert!(contains(&wasm,b"cooperate"),"await requires scheduler handoff");
    assert!(contains(&wasm,b"print_str"),"direct stdout requires typed host import");
    assert_eq!(wasm,compile(src).unwrap(),"deterministic guest emission");
}

#[test]
fn nonblocking_writes_remain_explicitly_unsupported_even_in_suspends_context() {
    for src in [
        r#"pub suspends fnc main(): int { nb stdio.out << "not yet"; return await 42; }"#,
        r#"pub suspends fnc main(): int { nb readch inbox >> val future; return await 42; }"#,
        r#"pub suspends fnc main(): int { nb outgoing << "hi"; return await 42; }"#,
    ] {
        let error=compile(src).expect_err("unsupported NB I/O must not emit Wasm");
        assert!(matches!(error.code,"E_UNSUPPORTED" | "E_IO_SYNTAX"),"{src}: {error}");
    }
}

#[test]
fn detached_io_cannot_inherit_jspsi_task_scheduler_without_owner_contract() {
    for src in [
        r#"pub suspends fnc main(): int { do writech stdio.out, "hi"; return await 42; }"#,
        r#"pub suspends fnc main(): int { do stdio.out << "hi"; return await 42; }"#,
        r#"pub suspends fnc main(): int { do readch stdio.in, |outcome| -> void {}; return await 42; }"#,
    ] {
        let error=compile(src).expect_err("supervised detached I/O is not runtime-lowered");
        assert_eq!(error.code,"E_UNSUPPORTED","{src}: {error}");
    }
}

#[test]
fn effect_boundaries_still_fail_closed_with_io_and_await() {
    for (src,code) in [
        (r#"pub fnc main(): int { return await 42; }"#,"E_EFFECT"),
        (r#"pub pure fnc main(): int { stdio.out << "effect"; return 42; }"#,"E_PURE"),
        (r#"pub pure suspends fnc main(): int { return await 42; }"#,"E_PURE"),
    ] {
        let diag=compile(src).expect_err(src);
        assert_eq!(diag.code,code,"{src}: {diag}");
    }
}
