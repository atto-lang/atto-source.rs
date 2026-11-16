//! Bounded compiled-guest admission for one copied int arm-local capture.
//! These tests only assert source compilation; browser execution is separate.
use atto_compiler::compile;

fn accepted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("capture guest rejected: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).expect("deterministic AOT capture guest"));
    for needle in [b"atto.async".as_slice(), b"task_start_i64", b"task_invoke_i64"] {
        assert!(wasm.windows(needle.len()).any(|w| w == needle), "missing capture ABI");
    }
}

fn rejected(source: &str, code: &str) {
    let diag = compile(source).expect_err("unsupported task capture MUST emit no Wasm");
    assert_eq!(diag.code, code, "unexpected diagnostic: {diag}");
}

#[test]
fn match_copies_int_adt_payload_into_task() {
    accepted(r#"
        fnc checked(): Option<int> { return Some(41); }
        pub suspends fnc main(): int {
            return match<int> checked() over
                on Some(n) : int -> await task { return n + 1; },
                on None : int -> 0,
            end;
        }
    "#);
}

#[test]
fn switch_copies_int_result_payload_into_task() {
    accepted(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub suspends fnc main(): int {
            return switch<int> checked() over
                case Err(message) : int -> 0,
                case Ok(value) : int -> await task { return value + 1; },
            end;
        }
    "#);
}

#[test]
fn captured_adt_or_string_payload_never_becomes_an_i64_handle() {
    rejected(r#"
        fnc checked(): Result<int,string> { return Err("no"); }
        pub suspends fnc main(): int {
            return match<int> checked() over
                on Err(error) : int -> await task { return error; },
                on Ok(value) : int -> value,
            end;
        }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        fnc nested(): Option<Result<int,string>> { return Some(Ok(42)); }
        pub suspends fnc main(): int {
            return switch<int> nested() over
                case Some(outcome) : int -> await task { return outcome; },
                case None : int -> 0,
            end;
        }
    "#, "E_UNSUPPORTED");
}

#[test]
fn unbound_non_arm_task_capture_still_fails_closed() {
    rejected(r#"
        pub suspends fnc main(): int {
            val x = 42;
            return await task { return x; };
        }
    "#, "E_UNBOUND");
}

#[test]
fn shadowing_arm_binder_inside_task_is_not_guessed() {
    rejected(r#"
        fnc checked(): Option<int> { return Some(41); }
        pub suspends fnc main(): int {
            return match<int> checked() over
                on Some(n) : int -> await task { val n = 77; return n; },
                on None : int -> 0,
            end;
        }
    "#, "E_UNSUPPORTED");
}
