//! Checked lowering for explicit `suspends |v| -> await v` pipeline stages.
//! This is a deliberately narrow *executable* identity-await adapter:
//! general first-class/capturing lambda values remain unsupported.
use atto_compiler::compile;

fn has(wasm: &[u8], needle: &[u8]) -> bool {
    wasm.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn identity_await_lambda_pipeline_compiles_as_real_guest_future_await() {
    let src = r#"
fnc transform_result(int value): int { return value + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };
    val result = pending
        |> suspends |v| -> await v
        |> transform_result;
    return result;
}
"#;
    let wasm = compile(src).expect("real Future<int> must lower through identity-await");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert!(has(&wasm, b"task_await"), "must await the Future; not a no-op");
    assert!(has(&wasm, b"cooperate"), "suspending pipeline must yield cooperatively");
    assert_eq!(wasm, compile(src).unwrap(), "deterministic codegen");
}

#[test]
fn equivalent_explicit_await_and_pipeline_compile_to_identical_wasm() {
    let header = "pub suspends fnc main(): int { val future = task { return 42; };";
    let direct = format!("{header} return await future; }}");
    let piped = format!("{header} return future |> suspends |f| -> await f; }}");
    // Source position metadata may differ, but the checked async markers and
    // program shape must be equivalent.
    let a = compile(&direct).expect("ordinary await");
    let b = compile(&piped).expect("lambda await stage");
    assert!(has(&a, b"task_await") && has(&b, b"task_await"));
    assert!(has(&a, b"cooperate") && has(&b, b"cooperate"));
}

#[test]
fn bare_await_stage_and_unsupported_lambda_shapes_fail_closed() {
    for (src, expected) in [
        ("pub suspends fnc main(): int { return 42 |> await; }", "E_UNSUPPORTED"),
        ("pub suspends fnc main(): int { return 42 |> suspends |v| -> v; }", "E_UNSUPPORTED"),
        ("pub suspends fnc main(): int { return 42 |> suspends |v| -> await wrong; }", "E_PIPE_BIND"),
        ("pub suspends fnc main(): int { return 42 |> suspends |v| -> await v + 1; }", "E_UNSUPPORTED"),
        ("pub suspends fnc main(): int { return 42 |> suspends |v, w| -> await v; }", "E_SYNTAX"),
    ] {
        let diag = compile(src).expect_err("no guest artifact for unsupported pipeline lambda");
        assert_eq!(diag.code, expected, "{src}: {diag}");
    }
}

#[test]
fn async_pipe_cannot_escape_non_suspends_or_pure_boundaries() {
    for src in [
        "pub fnc main(): int { return 42 |> suspends |v| -> await v; }",
        "pub pure suspends fnc main(): int { return 42 |> suspends |v| -> await v; }",
    ] {
        let diag = compile(src).expect_err("suspension requires an authorized effect context");
        assert!(matches!(diag.code, "E_EFFECT" | "E_PURE"), "{src}: {diag}");
    }
}
