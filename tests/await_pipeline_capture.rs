//! Executable terminal directional capture after the checked identity-await
//! pipeline stage. This is NOT an stdin/Readable<T> guest ABI.
use atto_compiler::compile;

fn compile_ok(src: &str) -> Vec<u8> {
    let guest = compile(src).expect("checked capture should compile to guest Wasm");
    assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    assert!(guest.windows(10).any(|b| b == b"task_await"), "must await the real Future");
    assert!(guest.windows(9).any(|b| b == b"cooperate"), "must suspend cooperatively");
    assert_eq!(guest, compile(src).unwrap(), "Wasm byte output must be deterministic");
    guest
}

#[test]
fn terminal_capture_from_future_is_real_checked_wasm() {
    for spelling in ["susp", "suspends"] {
        let src = format!(r#"
pub suspends fnc main(): int {{
    val pending = task {{ return 41; }};
    pending |> {spelling} |f| -> await f >> val answer: int;
    return answer + 1;
}}
"#);
        compile_ok(&src);
    }
}

#[test]
fn named_transform_followed_by_typed_capture_remains_executable() {
    let src = r#"
fnc transform_result(int x): int { return x + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };
    pending |> susp |f| -> await f |> transform_result >> val int answer;
    return answer;
}
"#;
    compile_ok(src);
}

#[test]
fn explicit_pipeline_terminal_capture_infers_result_and_supports_let() {
    let src = r#"
pub suspends fnc main(): int {
    val pending = task { return 41; };
    pending |> suspends |f| -> await f >> let result;
    result = result + 1;
    return result;
}
"#;
    compile_ok(src);
}

#[test]
fn capture_rejects_effect_leak_type_mismatch_double_consumption_and_ambiguity() {
    for (src, expected) in [
        ("pub fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x: int; return x; }", "E_EFFECT"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x: string; return 42; }", "E_TYPE"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> const x: int; return 42; }", "E_CONST_EFFECT"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await wrong >> val x:int; return 42; }", "E_PIPE_BIND"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x:int; return await p; }", "E_MOVED"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; await p >> val x:int; return x; }", "E_IO_SYNTAX"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x:int; p |> susp |v| -> await v >> val y:int; return x; }", "E_MOVED"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x:int; p |> susp |g| -> await g >> val x:int; return x; }", "E_MOVED"),
    ] {
        let diag=compile(src).expect_err("unsafe or ambiguous capture must not emit guest Wasm");
        assert_eq!(diag.code, expected, "{src}: {diag}");
    }
}

#[test]
fn direction_tokens_are_contiguous_and_binders_must_be_fresh() {
    for src in [
        "pub suspends fnc main(): int { val p=task{return 42;}; p |> susp |f| -> await f > > val x:int; return x; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; val x=0; p |> susp |f| -> await f >> val x:int; return x; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; p |> susp |f| -> await f >> val ; return 42; }",
    ] { compile(src).expect_err("malformed terminal direction must be rejected"); }
}
