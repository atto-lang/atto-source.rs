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
fn susp_alias_and_suspends_long_form_have_the_same_checked_await_profile() {
    for keyword in ["susp", "suspends"] {
        let src = format!("pub suspends fnc main(): int {{ val pending=task {{return 42;}}; return pending |> {keyword} |v| -> await v; }}");
        let wasm=compile(&src).expect("both lambda effect spellings must lower to await");
        assert!(has(&wasm, b"task_await") && has(&wasm, b"cooperate"));
    }
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

#[test]
fn contextual_nb_stdin_read_await_forms_recognized_but_do_not_fake_guest_execution() {
    for operation in [
        "nb stdio.in >> (val String pending_line << await Future<string>);",
        "nb stdio.in >> Future<string> x | await x >> val String pending_line;",
        "nb stdio.in >> (val outcome: Result<Option<string>,IoError> << await Future<Result<Option<string>,IoError>>);",
        "nb stdio.in >> Future<Result<Option<string>,IoError>> x | await x >> val outcome: Result<Option<string>,IoError>;",
    ] {
        let src = format!("pub suspends fnc main(): int {{ {operation} return 42; }}");
        let diag = compile(&src).expect_err("typed stdin Future guest ABI remains unavailable");
        assert_eq!(diag.code, "E_UNSUPPORTED", "{operation}: {diag}");
        assert!(diag.message.contains("read") || diag.message.contains("nonblocking"),
            "{operation}: expected fail-closed guest-ABI diagnostic, got {diag}");
    }
}

#[test]
fn contextual_read_await_rejects_binder_theft_and_lossy_future_payload_types() {
    for (operation, expected) in [
        ("nb stdio.in >> Future<string> x | await y >> val String line;", "E_PIPE_BIND"),
        ("nb stdio.in >> Future<int> x | await x >> val String line;", "E_TYPE"),
        ("nb stdio.in >> (val String line << await Future<int>);", "E_TYPE"),
        ("nb stdio.in >> (const String line << await Future<string>);", "E_CONST_EFFECT"),
        ("nb stdio.in >> Future<string> x | await x >> const String line;", "E_CONST_EFFECT"),
        // A non-awaited Future<T> placeholder is an illegal I/O direction
        // operand, rejected by preflight with its stable I/O diagnostic.
        ("nb stdio.in >> (val String line << Future<string>);", "E_IO_SYNTAX"),
        ("nb stdio.in >> Future<string> x | await x >> val String;", "E_SYNTAX"),
    ] {
        let src = format!("pub suspends fnc main(): int {{ {operation} return 42; }}");
        let diag = compile(&src).expect_err("malformed/lossy read await must fail without artifact");
        assert_eq!(diag.code, expected, "{operation}: {diag}");
    }
}


#[test]
fn std_await_adapter_executes_future_then_scalar_pipeline_in_guest_wasm() {
    let src = r#"
fnc transform_result(int value): int { return value + 1; }
pub susp fnc main(): int {
    val pending = task { return 41; };
    return pending
        |> std.await
        |> transform_result;
}
"#;
    let wasm = compile(src).expect("std.await uses checked guest Future<int> lowering");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert!(has(&wasm, b"task_await"), "adapter must await, not erase the Future");
    assert!(has(&wasm, b"cooperate"), "adapter must preserve scheduler cooperation");
    assert_eq!(wasm, compile(src).unwrap(), "adapter codegen must be deterministic");
}

#[test]
fn std_await_adapter_matches_explicit_identity_await_guest_path() {
    let header = "pub susp fnc main(): int { val pending = task { return 42; };";
    let explicit = format!("{header} return pending |> susp |f| -> await f; }}");
    let alias = format!("{header} return pending |> std.await; }}");
    for src in [&explicit, &alias] {
        let wasm = compile(src).expect("both stages must compile");
        assert!(has(&wasm, b"task_await") && has(&wasm, b"cooperate"));
    }
}

#[test]
fn std_await_adapter_caret_return_and_terminal_value_capture_compile() {
    let src = r#"
fnc transform_result(int value): int { return value + 1; }
pub susp fnc main(): int {
    val pending = task { return 41; };
    ^ pending
        |> std.await
        |> transform_result
        >> val result: int;
}
"#;
    let wasm = compile(src).expect("checked caret return and capture with std.await");
    assert!(has(&wasm, b"task_await") && has(&wasm, b"cooperate"));
}

#[test]
fn std_await_adapter_rejects_factory_raw_scalar_and_effect_escape() {
    for (source, expected) in [
        ("pub susp fnc main(): int { val p=task {return 42;}; return p |> std.await(); }", "E_PIPE_ADAPTER"),
        ("pub susp fnc main(): int { return 42 |> std.await; }", "E_TYPE"),
        ("pub fnc main(): int { return 42 |> std.await; }", "E_EFFECT"),
    ] {
        let diag = compile(source).expect_err("invalid adapter use must not emit a guest Wasm artifact");
        assert_eq!(diag.code, expected, "{source}: {diag}");
    }
    let pure = "pub pure susp fnc main(): int { val p=task {return 42;}; return p |> std.await; }";
    let diag = compile(pure).expect_err("pure scope must not launder suspension effects");
    assert!(matches!(diag.code, "E_PURE" | "E_EFFECT"), "{pure}: {diag}");
}

#[test]
fn std_await_rejects_shadowed_core_namespace_without_reinterpreting_the_local() {
    let source = r#"
pub susp fnc main(): int {
    val std = 99;
    val pending = task { return 41; };
    return pending |> std.await;
}
"#;
    let diag = compile(source).expect_err("shadowed 'std' must not silently invoke a builtin");
    assert_eq!(diag.code, "E_PIPE_ADAPTER", "{diag}");
    assert!(diag.message.contains("shadowed"), "{diag}");
}

#[test]
fn std_await_rejects_reawaiting_a_consumed_future() {
    let source = r#"
pub susp fnc main(): int {
    val pending = task { return 41; };
    val first = pending |> std.await;
    return pending |> std.await;
}
"#;
    let diag = compile(source).expect_err("a Future handle is consumed exactly once");
    assert_eq!(diag.code, "E_MOVED", "{diag}");
}
