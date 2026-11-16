//! ADT payload discard patterns: compile genuine Wasm, reject without artifacts.
use atto_compiler::compile;

fn accepted(source: &str) {
    let bytes = compile(source).unwrap_or_else(|e| panic!("unexpected {}: {}", e.code, e));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "missing Wasm binary header");
    assert_eq!(bytes, compile(source).expect("deterministic AOT lowering"));
}
fn rejected(source: &str, code: &str) {
    let diag = compile(source).expect_err("invalid pattern must not emit guest Wasm");
    assert_eq!(diag.code, code, "unexpected diagnostic: {diag}");
}

#[test]
fn err_empty_parens_discards_error_while_ok_binds_value() {
    accepted(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> checked() over
                on Err() : int -> 0,
                on Ok(value) : int -> value + 1,
            end;
        }
    "#);
}
#[test]
fn ok_empty_parens_and_explicit_wildcard_discard_success() {
    accepted(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return switch<int> checked() over
                case Err(_) : int -> -1,
                case Ok() : int -> 42,
            end;
        }
    "#);
    accepted(r#"
        fnc checked(): Result<int,string> { return Err("ignored"); }
        pub fnc main(): int {
            return match<int> checked() over
                on Err(_) : int -> 42,
                on Ok(_) : int -> 0,
            end;
        }
    "#);
}
#[test]
fn option_some_empty_and_wildcard_do_not_require_binding() {
    accepted(r#"
        fnc optional(): Option<int> { return Some(13); }
        pub fnc main(): int {
            return match<int> optional() over
                on Some() : int -> 42,
                on None : int -> 0,
            end;
        }
    "#);
    accepted(r#"
        fnc optional(): Option<Result<int,string>> { return Some(Ok(41)); }
        pub fnc main(): int {
            return switch<int> optional() over
                case Some(_) : int -> 42,
                case None : int -> 0,
            end;
        }
    "#);
}
#[test]
fn empty_pattern_never_invents_value_or_error_local() {
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> checked() over
                on Err() : int -> 0,
                on Ok() : int -> value + 1,
            end;
        }
    "#, "E_UNBOUND");
    rejected(r#"
        fnc checked(): Result<int,string> { return Err("ignored"); }
        pub fnc main(): int {
            return switch<int> checked() over
                case Err() : int -> error,
                case Ok(_) : int -> 0,
            end;
        }
    "#, "E_UNBOUND");
}
#[test]
fn constructors_require_payloads_and_none_rejects_parentheses() {
    rejected(r#"
        fnc checked(): Result<int,string> { return Ok(); }
        pub fnc main(): int { return 42; }
    "#, "E_ARITY");
    rejected(r#"
        fnc checked(): Result<int,string> { return Err(); }
        pub fnc main(): int { return 42; }
    "#, "E_ARITY");
    rejected(r#"
        fnc optional(): Option<int> { return Some(41); }
        pub fnc main(): int {
            return match<int> optional() over
                on Some() : int -> 42,
                on None() : int -> 0,
            end;
        }
    "#, "E_ARITY");
}
