//! Guarded ADT match must bind only inside the reached candidate.  The
//! original hoisted-setup lowering read every same-tag guard binder first.
//! These tests inspect typed IR as well as compiling real Wasm.
use atto_compiler::{check::{self, TExpr, TStmt, TE}, lexer, parser::Parser, compile};

fn typed(source: &str) -> check::TProgram {
    let toks = lexer::lex(source).expect("tokens");
    let ast = Parser::new(toks).parse_program().expect("syntax");
    check::check(&ast).expect("typed program")
}

fn valid(src: &str) {
    let wasm = compile(src).unwrap_or_else(|e| panic!("unexpected {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(src).expect("deterministic native-to-Wasm compiler"));
}
fn invalid(src: &str, code: &str) {
    let error = compile(src).expect_err("invalid source must emit no guest artifact");
    assert_eq!(error.code, code, "wrong error: {error}");
}

#[test]
fn typed_ir_defers_tagged_payload_load_until_reached_arm() {
    let program = typed(r#"
        fnc value(): Result<int,string> { return Ok(41); }
        pub fnc main(): int {
            return match<int> value() over
                on Ok(n) when n > 99 : int -> 1,
                on Ok(n) when n > 0 : int -> n + 1,
                on Ok(_) : int -> 2,
                on Err(error) when error == "bad" : int -> 3,
                on Err() : int -> 0,
            end;
        }
    "#);
    let main = &program.funcs[program.main as usize];
    let dispatch = main.body.iter().find_map(|s| match s {
        TStmt::Block(body) => Some(body),
        _ => None,
    }).expect("expected checked match block");
    assert_eq!(dispatch.len(), 2 + 5 + 1,
        "one subject evaluation, one selection flag, five linear candidates and a fail-closed check");
    assert!(matches!(&dispatch[0], TStmt::Set(_, _)));
    assert!(matches!(&dispatch[1], TStmt::Set(_, TE { e: TExpr::Bool(false), .. })));
    // Previously the hoisted prefix contained independent tagged payload
    // projections for every guarded arm. Those are now impossible here.
    let mut payload_candidates = 0;
    for (arm_index, candidate) in dispatch[2..7].iter().enumerate() {
        let TStmt::If(test, None) = candidate else {
            panic!("arm {arm_index} not conditional dispatch: {candidate:?}");
        };
        assert_eq!(test.len(), 1);
        let body = &test[0].1;
        if matches!(body.first(), Some(TStmt::Set(_, TE { e: TExpr::AdtPayload(_), .. }))) {
            payload_candidates += 1;
        }
        assert!(body.iter().any(|s| matches!(s, TStmt::If(..) | TStmt::Set(..))),
            "each candidate either evaluates its guard or selects a body");
    }
    assert_eq!(payload_candidates, 3, "only named tagged binders extract payloads");
    assert!(matches!(dispatch.last(), Some(TStmt::Unreachable)),
        "all-exiting match must retain its definite-return and fail-closed proof");
}

#[test]
fn guard_fallthrough_and_selected_body_preserve_order() {
    valid(r#"
        fnc observe(int n): bool {
            stdio.println("evaluated");
            return n > 0;
        }
        fnc value(): Option<int> { return Some(41); }
        pub fnc main(): int {
            return switch<int> value() over
                case Some(n) when n > 100 : int -> 0,
                case Some(n) when observe(n) : int -> n + 1,
                case Some() : int -> 1,
                case None : int -> 2,
            end;
        }
    "#);
    valid(r#"
        fnc value(): Result<int,string> { return Err("later"); }
        pub fnc main(): int {
            match value() over
                on Ok(n) when n > 0 -> { return 0; }
                on Err(error) when error == "later" -> { return 42; }
                on Err(_) -> { return 7; }
                on Ok() -> { return 3; }
            end
        }
    "#);
}

#[test]
fn checked_guards_do_not_imply_exhaustiveness_or_leak_binders() {
    invalid(r#"
        fnc value(): Result<int,string> { return Ok(42); }
        pub fnc main(): int {
            return match<int> value() over
                on Ok(n) when n > 10 : int -> 42,
                on Err(error) when error == "bad" : int -> 0,
            end;
        }
    "#, "E_MATCH");
    invalid(r#"
        fnc value(): Result<int,string> { return Ok(42); }
        pub fnc main(): int {
            return match<int> value() over
                on Ok(_) when n > 10 : int -> 42,
                on Ok() : int -> 0,
                on Err() : int -> 1,
            end;
        }
    "#, "E_UNBOUND");
    invalid(r#"
        fnc value(): Result<int,string> { return Ok(42); }
        pub fnc main(): int {
            return match<int> value() over
                on Ok(n) when n : int -> 42,
                on Ok() : int -> 0,
                on Err() : int -> 1,
            end;
        }
    "#, "E_TYPE");
}


#[test]
fn statement_match_without_definite_exit_preserves_continuation() {
    let src = r#"
        fnc optional(): Option<int> { return Some(41); }
        pub fnc main(): int {
            let result = 0;
            match optional() over
                on Some(n) when n > 40 -> { result = 42; }
                on Some(n) when n > 0 -> { result = 99; }
                on Some() -> { result = -1; }
                on None -> { result = 0; }
            end
            return result;
        }
    "#;
    let program = typed(src);
    let main = &program.funcs[program.main as usize];
    let dispatch = main.body.iter().find_map(|s| match s {
        TStmt::Block(body) if body.len() > 3 => Some(body),
        _ => None,
    }).expect("non-returning match block");
    assert!(matches!(dispatch.last(), Some(TStmt::If(_, None))),
        "statement match must trap only on impossible unmatched path");
    valid(src);
}
