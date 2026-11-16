//! ADT guard payloads must initialize before 'when', under a proven tag.
//! Run against real source-to-Wasm output; invalid programs produce no guest.
use atto_compiler::compile;

fn accepted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("unexpected {}: {}", e.code, e));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).expect("deterministic Wasm guest"));
}
fn rejected(source: &str, code: &str) {
    let e = compile(source).expect_err("invalid guard/match must emit no guest Wasm");
    assert_eq!(e.code, code, "unexpected diagnostic: {e}");
}

#[test]
fn guarded_result_payloads_bind_and_default_is_exhaustive() {
    accepted(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub suspends fnc main(): int {
            return match<int> checked() over
                on Err(error) when error != "E3" : int -> 0,
                on Ok(value) when value > 5 : int ->
                    await task { return value + 1; },
                default : int -> 10,
            end;
        }
    "#);
    accepted(r#"
        fnc checked(): Result<int,string> { return Err("E3"); }
        pub fnc main(): int {
            return match<int> checked() over
                on Err(error) when error != "E3" : int -> 0,
                on Ok(value) when value > 5 : int -> value + 1,
                default : int -> 10,
            end;
        }
    "#);
    accepted(r#"
        fnc checked(): Result<int,string> { return Err("E4"); }
        pub fnc main(): int {
            return match<int> checked() over
                on Err(error) when error != "E3" : int -> 0,
                on Ok(value) when value > 5 : int -> value + 1,
                default : int -> 10,
            end;
        }
    "#);
}

#[test]
fn guarded_option_and_repeated_variant_fallbacks_are_legal() {
    accepted(r#"
        fnc checked(): Option<int> { return Some(2); }
        pub fnc main(): int {
            return switch<int> checked() over
                case Some(n) when n < 5 : int -> 42,
                case Some(n) when n > 5 : int -> 99,
                case Some() : int -> 13,
                case None : int -> 0,
            end;
        }
    "#);
    accepted(r#"
        fnc checked(): Result<int,string> { return Ok(3); }
        pub fnc main(): int {
            return match<int> checked() over
                on Ok(value) when value > 5 : int -> 99,
                on Ok(value) when value < 5 : int -> 42,
                on Ok(_) : int -> 13,
                on Err() : int -> 0,
            end;
        }
    "#);
}

#[test]
fn guarded_arms_do_not_prove_exhaustiveness() {
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> checked() over
                on Ok(value) when value > 5 : int -> 42,
                on Err(error) when error == "E3" : int -> 0,
            end;
        }
    "#, "E_MATCH");
    rejected(r#"
        fnc checked(): Option<int> { return Some(2); }
        pub fnc main(): int {
            return switch<int> checked() over
                case Some(n) when n > 5 : int -> 99,
                case None : int -> 0,
            end;
        }
    "#, "E_MATCH");
}

#[test]
fn guards_are_bool_and_binders_are_arm_local() {
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> checked() over
                on Ok(value) when value : int -> 42,
                on Err() : int -> 0,
            end;
        }
    "#, "E_TYPE");
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> checked() over
                on Err(error) when error == "E3" : int -> 0,
                on Ok(value) when error == "E3" : int -> value + 1,
                default : int -> 10,
            end;
        }
    "#, "E_UNBOUND");
}

#[test]
fn an_unguarded_variant_makes_later_same_variant_unreachable() {
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> checked() over
                on Ok(value) : int -> value,
                on Ok(other) when other > 5 : int -> 99,
                on Err() : int -> 0,
            end;
        }
    "#, "E_MATCH");
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return switch<int> checked() over
                case Ok() : int -> 42,
                case Err() : int -> 0,
                default : int -> 10,
            end;
        }
    "#, "E_MATCH");
}
