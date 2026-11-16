//! Executable compiler-front-end regression cases for switch/case lowering.
//! Every accepted source must emit actual WebAssembly bytes; negative cases
//! must produce a compiler diagnostic and absolutely no guest artifact.
//! Does not establish Java/LLVM/.NET backend parity or first-class switch values.
use atto_compiler::compile;

fn accepted(src: &str) {
    let guest = compile(src).unwrap_or_else(|e| panic!("compiler rejected switch case: {e}"));
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"), "real Wasm header required");
    assert_eq!(guest, compile(src).expect("deterministic source lowering"));
}
fn rejected(src: &str, code: &str) {
    let e = compile(src).expect_err("invalid switch must not produce guest Wasm");
    assert_eq!(e.code, code, "unexpected diagnostic: {e}");
}

#[test]
fn typed_return_switch_and_binary_case_result() {
    accepted(r#"
        pub fnc main(): int {
            return switch<int> 41 over
                case 0 : int -> -1,
                case n when n > 0 : int -> n + 1,
                default : int -> 0,
            end;
        }
    "#);
    accepted(r#"
        pub fnc main(): int {
            return switch<int> true over
                case false -> 0,
                case true : int -> 6 * 7,
            end;
        }
    "#);
}

#[test]
fn tagged_nested_cases_share_match_exhaustiveness_rules() {
    accepted(r#"
        fnc checked(): Option<Result<int,string>> { return Some(Ok(41)); }
        pub fnc main(): int {
            return switch<int> checked() over
                case None : int -> 0,
                case Some(candidate) : int -> {
                    return switch<int> candidate over
                        case Err(error) : int -> -3,
                        case Ok(n) : int -> n + 1,
                    end;
                },
            end;
        }
    "#);
}

#[test]
fn switch_statement_and_do_switch_have_no_implicit_fallthrough() {
    accepted(r#"
        pub fnc main(): int {
            switch<int> true over
                case true : int -> { return 42; }
                default : int -> { return 0; }
            end
        }
    "#);
    accepted(r#"
        pub fnc main(): int {
            do switch false over {
                case false -> { return 42; }
                case true -> { return 0; }
            }
        }
    "#);
    // Java-compatible legacy untyped switch does not require 'over'.
    accepted(r#"
        pub fnc main(): int {
            switch 1
                case 1 -> { return 42; }
                default -> { return 0; }
            end
        }
    "#);
    // Alias 'on' is supported for shared AST tooling and existing examples.
    accepted(r#"
        pub fnc main(): int {
            switch false over
                on true -> { return 0; }
                on false -> { return 42; }
            end
        }
    "#);
}

#[test]
fn reject_wrong_case_result_and_header_type() {
    rejected(r#"
        pub fnc main(): int {
            return switch<int> true over
                case true : bool -> true,
                default : int -> 0,
            end;
        }
    "#, "E_TYPE");
    // Typed switch<T> never silently accepts the ambiguous legacy delimiter.
    rejected(r#"
        pub fnc main(): int {
            switch<int> true
                case true : int -> { return 42; }
                default : int -> { return 0; }
            end
        }
    "#, "E_SYNTAX");
    rejected(r#"
        pub fnc main(): int {
            switch<int> true over
                case true : bool -> { return 42; }
                default -> { return 0; }
            end
        }
    "#, "E_TYPE");
    rejected(r#"
        pub fnc main(): int {
            return switch<int> true over
                case true : int -> "wrong",
                default -> 42,
            end;
        }
    "#, "E_TYPE");
}

#[test]
fn reject_nonexhaustive_duplicate_and_malformed_cases() {
    rejected(r#"
        pub fnc main(): int {
            return switch<int> 2 over case 1 : int -> 41, end;
        }
    "#, "E_MATCH");
    rejected(r#"
        pub fnc main(): int {
            return switch<int> 1 over
                case 1 : int -> 42,
                case 1 : int -> 41,
                default : int -> 0,
            end;
        }
    "#, "E_MATCH");
    rejected(r#"
        pub fnc main(): int {
            return switch<int> true over
                case true : int -> 42,
                default : int -> 0,
                case false : int -> -1,
            end;
        }
    "#, "E_MATCH");
    rejected(r#"
        pub fnc main(): int {
            return switch<int> true over
                case true : -> 42,
                default -> 0,
            end;
        }
    "#, "E_SYNTAX");
    rejected(r#"
        pub fnc main(): int {
            return switch<int> true over
                case true : int => 42,
                default -> 0,
            end;
        }
    "#, "E_SYNTAX");
    rejected(r#"
        pub fnc main(): int {
            switch<int> true over
                case true : int -> { val x = 42; }
                default : int -> { return 0; }
            end
        }
    "#, "E_RETURN");
}
