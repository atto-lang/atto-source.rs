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
