//! A \`^ match\` returns from the enclosing callable, even if nested in
//! another match arm. It is not an infix XOR or a match-arm-only exit.
use atto_compiler::compile;

const COMPLEX: &str = include_str!("fixtures/pass/198-nested-caret-return-match.atto");

fn compile_checked(src:&str)->Vec<u8> {
    let guest=compile(src).unwrap_or_else(|e|panic!("{src}\n{e}"));
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(guest,compile(src).expect("deterministic guest"));
    guest
}

#[test]
fn caret_and_return_match_lower_to_exactly_the_same_guest() {
    let canonical=COMPLEX.replace("^ match", "return match");
    // Count executable return statements, not text inside documentation
    // comments mentioning the syntax.
    assert_eq!(
        COMPLEX.lines().filter(|line| line.trim_start().starts_with("^ match ")).count(),
        2,
    );
    let a=compile_checked(COMPLEX);
    let b=compile_checked(&canonical);
    assert_eq!(a,b,"^/return must share parser, match coverage and Wasm lowering");
    assert!(!a.windows(b"atto.channel_int".len()).any(|s|s==b"atto.channel_int"));
}

#[test]
fn four_level_nested_caret_match_is_definitely_returning() {
    compile_checked(r#"
pub fnc main(): int {
    val first: Option<int> = Some(41);
    ^ match first over
        on None -> { return -1; }
        on Some(n) -> {
            val second: Option<int> = Some(n);
            ^ match second over
                on None -> { return -2; }
                on Some(m) -> {
                    val third: Option<int> = Some(m);
                    ^ match third over
                        on None -> { return -3; }
                        on Some(p) -> {
                            val last: Option<int> = Some(p);
                            ^ match last over
                                on None -> { return -4; }
                                on Some(q) -> { return q + 1; }
                            end
                        }
                    end
                }
            end
        }
    end
}
"#);
}

#[test]
fn nested_caret_match_is_allowed_in_a_guard_failure_branch() {
    compile_checked(r#"
pub fnc main(): int {
    guard false {
        val result: Option<int> = Some(41);
        ^ match result over
            on None -> { return 0; }
            on Some(n) -> { return n + 1; }
        end
    }
    return 7;
}
"#);
}

#[test]
fn nested_match_must_still_prove_exhaustiveness() {
    let bad=r#"
pub fnc main(): int {
    val result: Option<int> = Some(41);
    ^ match result over
        on Some(n) -> {
            ^ match result over
                on Some(value) -> { return value; }
            end
        }
        on None -> { return -1; }
    end
}
"#;
    let e=compile(bad).expect_err("nested incomplete match must fail closed");
    assert_eq!(e.code,"E_MATCH","{e}");
}

#[test]
fn nested_caret_match_rejects_nonterminal_and_wrong_type_arms() {
    let no_return=r#"
pub fnc main(): int {
    val x: Option<int> = Some(41);
    ^ match x over
        on None -> { return 0; }
        on Some(n) -> {
            ^ match x over
                on None -> { return 0; }
                on Some(m) -> { val not_returned = m; }
            end
        }
    end
}
"#;
    let e=compile(no_return).expect_err("nonterminal nested arm");
    assert_eq!(e.code,"E_RETURN","{e}");

    let wrong_type=r#"
pub fnc main(): int {
    val x: Option<int> = Some(41);
    ^ match x over
        on None -> { return 0; }
        on Some(n) -> {
            return match x over
                on None -> { return 0; }
                on Some(m) -> { ^ "wrong"; }
            end
        }
    end
}
"#;
    let e=compile(wrong_type).expect_err("nested caret wrong return type");
    assert_eq!(e.code,"E_TYPE","{e}");
}

#[test]
fn caret_match_is_return_only_not_initializer_or_infix_operator() {
    for src in [
        "pub fnc main(): int { val x = ^ match 1 over on _ -> 42, end; return x; }",
        "pub fnc main(): int { return 41 ^ match 1 over on _ -> 1, end; }",
        "pub fnc main(): int { ^ ^ match 1 over on _ -> 42, end; }",
    ] {
        assert!(compile(src).is_err(),"invalid ^ match context emitted Wasm: {src}");
    }
}
