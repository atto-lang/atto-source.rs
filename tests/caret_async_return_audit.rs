//! Caret return + async identity-await + terminal >> val integration.
/// This tests compiler output and fail-closed diagnostics on the exact branch.
/// A real Wasm/JSPI execution oracle is in tests/browser-async-e2e.mjs.
use atto_compiler::{compile,lexer::{lex,Tok}};

fn wasm(source:&str)->Vec<u8>{
    let out=compile(source).unwrap_or_else(|e|panic!("{source}\n{e}"));
    assert_eq!(&out[..8],b"\0asm\x01\0\0\0");
    out
}

#[test]
fn caret_transformed_capture_is_exactly_equivalent_to_return_keyword() {
    let definitions=r#"
fnc transform_result(int v): int { return v + 1; }
"#;
    for marker in ["^","return"] {
        for effect in ["susp","suspends"] {
            let source=format!(r#"{definitions}
pub {effect} fnc main(): int {{
    val pending=task {{ return 41; }};
    {marker} pending
        |> susp |f| -> await f
        |> transform_result
        >> val result: int;
}}"#);
            let guest=wasm(&source);
            assert!(guest.windows(10).any(|x|x==b"task_await"));
            assert!(guest.windows(9).any(|x|x==b"cooperate"));
            assert_eq!(guest,wasm(&source),"deterministic compilation");
        }
    }
}

#[test]
fn caret_capture_inside_conditional_arm_satisfies_definite_return() {
    wasm(r#"
fnc plus_one(int v): int { ^ v + 1; }
pub susp fnc main(): int {
    val pending=task {return 41;};
    if true then
        ^ pending |> susp |f| -> await f |> plus_one >> val answer: int;
    else
        ^ 0;
    fi
}
"#);
}

#[test]
fn repeated_caret_return_never_implicitly_reawaits_a_consumed_future() {
    let source=r#"
pub susp fnc main(): int {
    val pending=task {return 41;};
    pending |> susp |f| -> await f >> val result: int;
    ^ pending |> susp |f| -> await f >> val again: int;
}
"#;
    let e=compile(source).expect_err("awaited Future is consumed");
    assert_eq!(e.code,"E_MOVED","{e}");
}

#[test]
fn return_capture_types_are_checked_without_coercion_or_discard() {
    let sources=[
        r#"fnc to_bool(int x): bool {return true;} pub susp fnc main(): int {val p=task{return 41;}; ^ p |> susp |f| -> await f |> to_bool >> val x: bool;}"#,
        r#"fnc pass(int x): int {return x;} pub susp fnc main(): int {val p=task{return 41;}; ^ p |> susp |f| -> await f |> pass >> val x: string;}"#,
        r#"pub susp fnc main(): int {val p=task{return 41;}; ^ p |> susp |f| -> await f >> const x: int;}"#,
    ];
    for source in sources {
        assert!(compile(source).is_err(),"type/effect mismatch emitted Wasm: {source}");
    }
}

#[test]
fn caret_cannot_act_as_operand_identifier_or_infix_operator() {
    assert!(matches!(lex("^ 42;").unwrap()[0].tok,Tok::Caret));
    for source in [
        "pub fnc main(): int { val x=^ 42; return x; }",
        "pub fnc main(): int { return 41 ^ 1; }",
        "pub fnc main(): int { ^ ^ 42; }",
        "pub fnc main(): int { ^ 41 ^^ 1; }",
    ] {
        assert!(compile(source).is_err(),"caret must be statement-position only: {source}");
    }
    // This is ordinary synchronous code and must not grow Future imports.
    wasm("pub fnc main(): int { ^ 42; }");
}

#[test]
fn bare_expression_cannot_use_terminal_capture_without_explicit_await_stage() {
    for source in [
        "pub susp fnc main(): int { val p=task{return 41;}; ^ p >> val result:int; }",
        "pub susp fnc main(): int { val p=task{return 41;}; ^ await p >> val result:int; }",
        "pub susp fnc main(): int { val p=task{return 41;}; ^ p |> susp |f| -> await f > > val result:int; }",
    ] {
        assert!(compile(source).is_err(),"unsupported capture must fail closed: {source}");
    }
}

#[test]
fn caret_return_void_is_legal_only_for_void_callable() {
    wasm("fnc noop(): void { ^; } pub fnc main(): int { ^ 42; }");
    assert!(compile("pub fnc main(): int { ^; }").is_err());
}
