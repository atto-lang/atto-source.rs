//! Executable Rust/Wasm frontend admission for the narrow typed-return-arm
//! subset. Task/await and expression-position match require separate runtime
//! integration; passing these tests MUST NOT be reported as backend parity.
use atto_compiler::compile;

fn accepted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!(
        "unexpected {}: {} at {:?}", e.code, e.message, e.pos));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "expected genuine Wasm emission");
}

fn rejected(source: &str, code: &str) {
    match compile(source) {
        Ok(_) => panic!("unexpected guest Wasm for a rejected program"),
        Err(e) => assert_eq!(e.code, code, "unexpected diagnostic: {e:?}"),
    }
}

#[test]
fn typed_return_match_and_adt_payloads() {
    accepted(r#"
        fnc result(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> result() over
                on Err(message) : int -> -3,
                on Ok(n) : int -> n + 1,
            end;
        }
    "#);
    accepted(r#"
        pub fnc main(): int {
            return match<int> 7 over
                on 0 -> 0,
                on n when n > 0 : int -> n + 35,
                on _ -> -3,
            end;
        }
    "#);
}

#[test]
fn nested_typed_return_match_and_definite_exit_blocks() {
    accepted(r#"
        fnc nested(): Option<Result<int,string>> { return Some(Ok(41)); }
        pub fnc main(): int {
            return match<int> nested() over
                on Some(value) : int -> {
                    return match<int> value over
                        on Err(message) : int -> -3,
                        on Ok(n) : int -> n + 1,
                    end;
                }
                on None : int -> 0,
            end;
        }
    "#);
    accepted(r#"
        pub fnc main(): int {
            match true over
                on true : int -> { return 42; }
                on false : int -> { return 0; }
            end
        }
    "#);
}

#[test]
fn type_conflicts_are_not_ignored() {
    rejected(r#"
        pub fnc main(): int {
            return match<int> 1 over on 1 : bool -> true, on _ : int -> 0, end;
        }
    "#, "E_TYPE");
    rejected(r#"
        pub fnc main(): int {
            return match<bool> true over
                on true : bool -> true,
                on false : bool -> false,
            end;
        }
    "#, "E_TYPE");
    rejected(r#"
        pub fnc main(): int {
            match true over
                on true : bool -> { return 42; }
                on false -> { return 0; }
            end
        }
    "#, "E_TYPE");
}

#[test]
fn typed_arm_must_exit_and_bad_annotation_fails_closed() {
    rejected(r#"
        pub fnc main(): int {
            match true over
                on true : int -> { val x = 42; }
                on false -> { return 0; }
            end
        }
    "#, "E_RETURN");
    rejected(r#"
        pub fnc main(): int {
            return match<int> 1 over on 1 : -> 42, on _ -> 0, end;
        }
    "#, "E_SYNTAX");
    rejected(r#"
        pub fnc main(): int {
            return match<int> 1 over on 1 : int => 42, on _ -> 0, end;
        }
    "#, "E_SYNTAX");
}
