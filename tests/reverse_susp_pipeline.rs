//! Checked right-associated reverse functional pipeline and declaration-first
//! single-result capture. This is NOT a stream pump or arbitrary I/O read.
use atto_compiler::{ast::{BindKind, ExprKind, Stmt, Type}, compile, lexer, parser::Parser};

fn compiled_guest(source: &str) -> Vec<u8> {
    let guest = compile(source).expect("the checked pipeline must compile to Wasm");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    assert!(guest.windows(10).any(|w| w == b"task_await"), "real Future await required");
    assert!(guest.windows(9).any(|w| w == b"cooperate"), "await must suspend cooperatively");
    assert_eq!(guest, compile(source).unwrap(), "deterministic Wasm guest");
    guest
}

const EXACT_EXAMPLE: &str = r#"
fnc transform_result(int x): int { ^ x + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };
    val result: int <<
        transform_result <|
        susp |f| -> await f <|
        pending;
    guard result > 0 { ^ -1; }
    ^ result;
}
"#;

#[test]
fn exact_user_example_is_a_real_wasm_await_and_guard() {
    compiled_guest(EXACT_EXAMPLE);
    let program = Parser::new(lexer::lex(EXACT_EXAMPLE).unwrap()).parse_program().unwrap();
    let main = program.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { name, kind, ty, init, .. } = &main.body[1] else {
        panic!("reverse << capture must lower to a normal local binding");
    };
    assert_eq!(name, "result");
    assert_eq!(*kind, BindKind::Val);
    assert_eq!(*ty, Some(Type::Int));
    assert!(matches!(&init.kind, ExprKind::Call(name, args)
        if name == "transform_result" && matches!(args.first().map(|x| &x.kind), Some(ExprKind::Await(_)))));
}

#[test]
fn both_effect_spellings_and_unparenthesized_line_breaks_work() {
    for keyword in ["susp", "suspends"] {
        let src = format!(r#"
fnc transform_result(int x): int {{ return x + 1; }}
pub suspends fnc main(): int {{
    val pending = task {{ return 41; }};
    let result: int <<
        transform_result <|
        {keyword} |f| -> await f <|
        pending;
    return result;
}}
"#);
        compiled_guest(&src);
    }
}

#[test]
fn direction_is_only_source_reordering_not_effect_reordering() {
    let forward = r#"
fnc transform_result(int x): int { return x + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };
    pending |> susp |f| -> await f |> transform_result >> val result: int;
    return result;
}
"#;
    let reverse = r#"
fnc transform_result(int x): int { return x + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };
    val result: int << transform_result <| susp |f| -> await f <| pending;
    return result;
}
"#;
    compiled_guest(forward);
    compiled_guest(reverse);
    let forward_program = Parser::new(lexer::lex(forward).unwrap()).parse_program().unwrap();
    let reverse_program = Parser::new(lexer::lex(reverse).unwrap()).parse_program().unwrap();
    let forward_main = forward_program.funcs.iter().find(|f| f.name == "main").unwrap();
    let reverse_main = reverse_program.funcs.iter().find(|f| f.name == "main").unwrap();
    let (Stmt::Bind { init: a, .. }, Stmt::Bind { init: b, .. }) =
        (&forward_main.body[1], &reverse_main.body[1]) else {
            panic!("both forms must produce ordinary Bind statements");
        };
    assert!(matches!(&a.kind, ExprKind::Call(name,args) if name == "transform_result"
        && matches!(args.first().map(|x| &x.kind), Some(ExprKind::Await(_)))));
    assert!(matches!(&b.kind, ExprKind::Call(name,args) if name == "transform_result"
        && matches!(args.first().map(|x| &x.kind), Some(ExprKind::Await(_)))));
}

#[test]
fn reverse_pipeline_is_right_associative_even_without_suspension() {
    let source = r#"
fnc twice(int x): int { return x * 2; }
fnc add_three(int x): int { return x + 3; }
pub fnc main(): int {
    val result = twice <| add_three <| 4;
    return result;
}
"#;
    let parsed = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &main.body[0] else { panic!("missing binding") };
    assert!(matches!(&init.kind, ExprKind::Call(name,args) if name == "twice"
        && matches!(args.first().map(|a| &a.kind),Some(ExprKind::Call(g,_)) if g == "add_three")));
    assert!(compile(source).is_ok(), "pure reverse application must also work");
}

#[test]
fn leading_reverse_await_stage_can_bind_directly() {
    let source = r#"
pub suspends fnc main(): int {
    val pending = task { return 42; };
    val result: int << susp |f| -> await f <| pending;
    return result;
}
"#;
    compiled_guest(source);
}

#[test]
fn rejects_wrong_effect_binder_types_const_and_repeated_future() {
    for (source, expected) in [
        ("pub fnc main(): int { val p=task{return 41;}; val result:int << susp |f| -> await f <| p; return result; }", "E_EFFECT"),
        ("pub suspends fnc main(): int { val p=task{return 41;}; val result:string << susp |f| -> await f <| p; return 0; }", "E_TYPE"),
        ("pub suspends fnc main(): int { val p=task{return 41;}; const result:int << susp |f| -> await f <| p; return 0; }", "E_CONST_EFFECT"),
        ("pub suspends fnc main(): int { val p=task{return 41;}; val result:int << susp |f| -> await g <| p; return 0; }", "E_PIPE_BIND"),
        ("pub suspends fnc main(): int { val p=task{return 41;}; val result:int << susp |f| -> await f <| p; return await p; }", "E_MOVED"),
        ("pub suspends fnc main(): int { val p=task{return 41;}; val result:int << p; return result; }", "E_UNSUPPORTED"),
    ] {
        let error = compile(source).expect_err("invalid reverse capture must not emit guest Wasm");
        assert_eq!(error.code, expected, "{source}: {error}");
    }
}

#[test]
fn rejects_unsafe_or_ambiguous_reverse_pipeline_forms() {
    let cases = [
        // The new operator is contiguous and is function application only.
        "fnc f(int x): int { return x; } pub fnc main(): int { return f < | 2; }",
        "fnc f(int x): int { return x; } pub fnc main(): int { return f <|; }",
        "fnc f(int x): int { return x; } pub fnc main(): int { return 2 |> f <| 2; }",
        "fnc f(int x): int { return x; } pub fnc main(): int { return f <| 2 |> f; }",
        "fnc f(int x): int { return x; } pub fnc main(): int { val x: int << f <| 2; return x; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; val result:int < < susp |f| -> await f <| p; return result; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; val result:int << susp |f| -> await f + 1 <| p; return result; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; val result:int << susp |f| -> await f < | p; return result; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; val result:int << susp |f| -> await f <| p <| p; return result; }",
    ];
    for source in cases {
        assert!(compile(source).is_err(), "ambiguous or malformed reverse pipeline: {source}");
    }
}

#[test]
fn reverse_capture_cannot_bypass_live_borrow_check() {
    let source = r#"
define class Parcel as
    pub let int value = 1;
end
pub suspends fnc main(): int {
    let parcel = new Parcel();
    val loan = borrow! parcel;
    val p = task { return 41; };
    val result: int << susp |f| -> await f <| p;
    return result + loan.value;
}
"#;
    let err = compile(source).expect_err("loan across a suspend must be rejected");
    assert_eq!(err.code, "E_BORROW");
}
