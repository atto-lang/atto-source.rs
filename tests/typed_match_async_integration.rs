//! Stacked Rust/Wasm JSPI integration candidate: typed arms with await task.
//! Host compilation checks do not establish browser execution or parity.
use atto_compiler::compile;

fn has(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn typed_awaited_task_match_emits_async_guest_abi() {
    let source = r#"
        pub suspends fnc main(): int {
            return match<int> 1 over
                on 1 : int -> await task { return 42; },
                on _ : int -> await task { return 0; },
            end;
        }
    "#;
    let wasm = compile(source).unwrap_or_else(|e| panic!("typed await task match: {e}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    for marker in [b"atto.async".as_slice(), b"task_start", b"task_await", b"cooperate"] {
        assert!(has(&wasm, marker), "missing async ABI marker {:?}", marker);
    }
    assert_eq!(wasm, compile(source).unwrap(), "nondeterministic typed arm Wasm");
}

#[test]
fn bad_task_types_and_unmarked_suspension_fail_closed() {
    let tests = [
        (r#"pub suspends fnc main(): int {
            return match<int> 1 over
                on 1 : int -> task { return 42; },
                on _ : int -> 0,
            end;
        }"#, "E_TYPE"),
        (r#"pub fnc main(): int {
            return match<int> 1 over
                on 1 : int -> await task { return 42; },
                on _ : int -> 0,
            end;
        }"#, "E_EFFECT"),
        (r#"pub suspends fnc main(): int {
            val x = 42;
            return match<int> 1 over
                on 1 : int -> await task { return x; },
                on _ : int -> 0,
            end;
        }"#, "E_UNBOUND"),
    ];
    for (source, wanted) in tests {
        let diag = compile(source).expect_err(source);
        assert_eq!(diag.code, wanted, "{source}: {diag}");
    }
}

#[test]
fn typed_awaited_task_switch_emits_same_checked_guest_abi() {
    let source = r#"
        pub suspends fnc main(): int {
            return switch<int> 1 over
                case 1 : int -> await task { return 42; },
                default : int -> await task { return 0; },
            end;
        }
    "#;
    let wasm = compile(source).unwrap_or_else(|e| panic!("typed await task switch: {e}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    for marker in [b"atto.async".as_slice(), b"task_start", b"task_await", b"cooperate"] {
        assert!(has(&wasm, marker), "missing switch async ABI marker {:?}", marker);
    }
    assert_eq!(wasm, compile(source).unwrap(), "nondeterministic typed switch Wasm");
}

#[test]
fn switch_task_effects_and_scope_are_checked_not_coerced() {
    let bad = [
        (r#"pub suspends fnc main(): int {
            return switch<int> 1 over
                case 1 : int -> task { return 42; },
                default : int -> 0,
            end;
        }"#, "E_TYPE"),
        (r#"pub fnc main(): int {
            return switch<int> 1 over
                case 1 : int -> await task { return 42; },
                default : int -> 0,
            end;
        }"#, "E_EFFECT"),
        (r#"pub suspends fnc main(): int {
            val x = 42;
            return switch<int> 1 over
                case 1 : int -> await task { return x; },
                default : int -> 0,
            end;
        }"#, "E_UNBOUND"),
        (r#"pub suspends fnc main(): int {
            return switch<int> 1 over
                case 1 : bool -> await task { return 42; },
                default : int -> 0,
            end;
        }"#, "E_TYPE"),
    ];
    for (source, expected) in bad {
        let diagnostic = compile(source).expect_err("unsupported switch must not emit Wasm");
        assert_eq!(diagnostic.code, expected, "{source}: {diagnostic}");
    }
}

#[test]
fn adt_payload_task_capture_uses_checked_copied_i64_frame() {
    let source = r#"
        fnc choice(): Result<int,string> { return Ok(41); }
        pub suspends fnc main(): int {
            return switch<int> choice() over
                case Err(error) : int -> -3,
                case Ok(value) : int -> await task { return value + 1; },
            end;
        }
    "#;
    let guest = compile(source).expect("one copied int case binder is supported in AOT task ABI v3");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    for marker in [b"task_start_i64".as_slice(), b"task_invoke_i64", b"task_await"] {
        assert!(has(&guest, marker), "missing checked i64 task ABI marker");
    }
    assert_eq!(guest, compile(source).expect("deterministic copied-int task guest"));

    // Non-int payload capture remains explicitly unsupported. In particular,
    // a string must never be reinterpreted as a forged i64 handle/pointer.
    let bad = r#"
        fnc choice(): Result<int,string> { return Err("bad"); }
        pub suspends fnc main(): int {
            return match<int> choice() over
                on Err(error) : int -> await task { return error; },
                on Ok(value) : int -> value,
            end;
        }
    "#;
    let diagnostic = compile(bad).expect_err("string payload capture must not emit guest Wasm");
    assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{diagnostic}");
}

#[test]
fn adt_case_without_task_capture_still_lowers_real_async_guest() {
    let source = r#"
        fnc choice(): Result<int,string> { return Ok(41); }
        pub suspends fnc main(): int {
            return switch<int> choice() over
                case Err(error) : int -> -3,
                case Ok(value) : int -> await task { return 42; },
            end;
        }
    "#;
    let guest = compile(source).unwrap_or_else(|e| panic!("adt awaited case: {e}"));
    assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    assert!(has(&guest, b"task_start"));
    assert!(has(&guest, b"task_await"));
    assert_eq!(guest, compile(source).unwrap());
}


#[test]
fn task_call_is_not_a_structured_task_block() {
    let rejected = r#"pub suspends fnc main(): int {
        return await task();
    }"#;
    let e = compile(rejected).expect_err("task() must not silently become a spawned task");
    assert_eq!(e.code, "E_UNSUPPORTED", "unexpected task-call diagnostic: {e}");
}
