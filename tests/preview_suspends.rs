//! Exact public /play suspends preview executes as a real JSPI Wasm guest.
//! The file must keep its byte-identical website source, not a simplified
//! stand-in; compile-only checks alone do not certify guest execution.
use atto_compiler::compile;
const SOURCE: &str = include_str!("playground_previews/suspends.atto");
fn good(src: &str) -> Vec<u8> {
    let w = compile(src).unwrap_or_else(|e| panic!("source must compile: {e}"));
    assert!(w.starts_with(b"\0asm\x01\0\0\0"));
    assert!(w.windows(b"atto.async".len()).any(|b| b == b"atto.async"));
    assert!(w.windows(b"cooperate".len()).any(|b| b == b"cooperate"));
    assert_eq!(w, compile(src).unwrap(), "byte-deterministic guest");
    w
}
fn bad(src: &str, expected: &str) {
    let d = compile(src).expect_err("invalid suspends lambda must emit no guest");
    assert_eq!(d.code, expected, "{d}");
}
#[test]
fn exact_website_suspends_compiles_real_jspsi_guest() {
    let guest = good(SOURCE);
    assert!(guest.windows(b"task_await".len()).any(|b| b==b"task_await"));
}
#[test]
fn suspending_lambda_call_requires_an_explicit_await() {
    bad(&SOURCE.replace("return await delayed(41);", "return delayed(41);"), "E_EFFECT");
}
#[test]
fn suspending_lambda_captures_cannot_escape_as_unverified_environment() {
    bad(r#"
       pub suspends fnc main(): int {
           val offset = 1;
           val delayed = suspends |int n| -> n + offset;
           return await delayed(41);
       }
    "#, "E_UNBOUND");
    bad(r#"
       pub suspends fnc main(): int {
           val delayed = suspends |int n| -> n + 1;
           val escaped = delayed;
           return await delayed(41);
       }
    "#, "E_UNSUPPORTED");
}
#[test]
fn direct_int_suspending_lambda_is_bounded_and_cannot_be_forged() {
    good("pub suspends fnc main(): int { val inc = susp |int n| -> n + 1; return await inc(41); }");
    bad("pub suspends fnc main(): int { val f = suspends |bool n| -> n; return await f(true); }", "E_UNSUPPORTED");
    bad("pub suspends fnc main(): int { val f = suspends |int n| -> n; val g = suspends |int x| -> x; return await f(42); }", "E_UNSUPPORTED");
}
#[test]
fn ordinary_callable_cannot_execute_awaited_susp_lambda() {
    bad("pub fnc main(): int { val f = suspends |int n| -> n + 1; return await f(41); }", "E_EFFECT");
}
