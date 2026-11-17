//! Executable terminal directional capture after the checked identity-await
//! pipeline stage. This is NOT an stdin/Readable<T> guest ABI.
use atto_compiler::compile;

fn compile_ok(src: &str) -> Vec<u8> {
    let guest = compile(src).expect("checked capture should compile to guest Wasm");
    assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    assert!(guest.windows(10).any(|b| b == b"task_await"), "must await the real Future");
    assert!(guest.windows(9).any(|b| b == b"cooperate"), "must suspend cooperatively");
    assert_eq!(guest, compile(src).unwrap(), "Wasm byte output must be deterministic");
    guest
}

#[test]
fn terminal_capture_from_future_is_real_checked_wasm() {
    for spelling in ["susp", "suspends"] {
        let src = format!(r#"
pub suspends fnc main(): int {{
    val pending = task {{ return 41; }};
    pending |> {spelling} |f| -> await f >> val answer: int;
    return answer + 1;
}}
"#);
        compile_ok(&src);
    }
}

#[test]
fn named_transform_followed_by_typed_capture_remains_executable() {
    let src = r#"
fnc transform_result(int x): int { return x + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };
    pending |> susp |f| -> await f |> transform_result >> val int answer;
    return answer;
}
"#;
    compile_ok(src);
}

#[test]
fn explicit_pipeline_terminal_capture_infers_result_and_supports_let() {
    let src = r#"
pub suspends fnc main(): int {
    val pending = task { return 41; };
    pending |> suspends |f| -> await f >> let result;
    result = result + 1;
    return result;
}
"#;
    compile_ok(src);
}

#[test]
fn capture_rejects_effect_leak_type_mismatch_double_consumption_and_ambiguity() {
    for (src, expected) in [
        ("pub fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x: int; return x; }", "E_EFFECT"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x: string; return 42; }", "E_TYPE"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> const x: int; return 42; }", "E_CONST_EFFECT"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await wrong >> val x:int; return 42; }", "E_PIPE_BIND"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x:int; return await p; }", "E_MOVED"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; await p >> val x:int; return x; }", "E_IO_SYNTAX"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x:int; p |> susp |v| -> await v >> val y:int; return x; }", "E_MOVED"),
        ("pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >> val x:int; p |> susp |g| -> await g >> val x:int; return x; }", "E_MOVED"),
    ] {
        let diag=compile(src).expect_err("unsafe or ambiguous capture must not emit guest Wasm");
        assert_eq!(diag.code, expected, "{src}: {diag}");
    }
}

#[test]
fn direction_tokens_are_contiguous_and_binders_must_be_fresh() {
    for src in [
        "pub suspends fnc main(): int { val p=task{return 42;}; p |> susp |f| -> await f > > val x:int; return x; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; val x=0; p |> susp |f| -> await f >> val x:int; return x; }",
        "pub suspends fnc main(): int { val p=task{return 42;}; p |> susp |f| -> await f >> val ; return 42; }",
    ] { compile(src).expect_err("malformed terminal direction must be rejected"); }
}



#[test]
fn caret_pipeline_capture_returns_transformed_future_without_an_extra_return() {
    let source = r#"
fnc transform_result(int x): int { return x + 1; }
pub suspends fnc main(): int {
    val pending = task { return 41; };

    ^ pending
        |> susp |f| -> await f
        |> transform_result
        >> val result: int;
}
"#;
    compile_ok(source);
}

#[test]
fn caret_and_return_capture_are_ast_equivalent_checked_bind_then_return() {
    use atto_compiler::{lexer, parser::Parser, ast::{Stmt, ExprKind, BindKind}};
    for marker in ["^", "return"] {
        let src=format!(r#"
fnc transform_result(int v): int {{ return v + 1; }}
pub suspends fnc main(): int {{
    val pending = task {{ return 41; }};
    {marker} pending |> suspends |f| -> await f
       |> transform_result >> val answer: int;
}}
"#);
        let p=Parser::new(lexer::lex(&src).unwrap()).parse_program().unwrap();
        let last=p.funcs.iter().find(|f|f.name=="main").unwrap().body.last().unwrap();
        let Stmt::Block(steps)=last else {panic!("return capture must become a checked block: {marker}")};
        assert_eq!(steps.len(),2);
        let Stmt::Bind {name,kind,ty,init,..}=&steps[0] else {panic!("binding missing")};
        assert_eq!(name,"answer");
        assert_eq!(*kind,BindKind::Val);
        assert_eq!(*ty,Some(atto_compiler::ast::Type::Int));
        assert!(matches!(&init.kind,ExprKind::Call(_,args)
            if matches!(args.first().map(|e|&e.kind),Some(ExprKind::Await(_)))));
        assert!(matches!(&steps[1],Stmt::Return(Some(e),_) if
            matches!(&e.kind,ExprKind::Var(v) if v=="answer")));
        compile_ok(&src);
    }
}

#[test]
fn caret_return_is_a_statement_marker_not_an_expression_or_an_operator() {
    use atto_compiler::lexer::{lex,Tok};
    assert!(matches!(lex("^ 42;").unwrap()[0].tok,Tok::Caret));
    for src in [
        "pub fnc main(): int { ^ 42 ^ 3; }",
        "pub fnc main(): int { val x = ^ 42; return x; }",
        "pub fnc main(): int { ^ ^ 42; }",
        "pub fnc main(): int { ^ 41 ^^ true; }",
    ] {
        assert!(compile(src).is_err(),"infix/nested caret forbidden: {src}");
    }
    // Ordinary caret return is not an async Future program; do not require
    // task_await or cooperate imports in its guest Wasm.
    let guest=compile("pub fnc main(): int { ^ 42; }")
        .expect("ordinary caret return must compile");
    assert_eq!(&guest[..8],b"\0asm\x01\0\0\0");
}

#[test]
fn caret_capture_checks_type_effects_and_invalid_binders() {
    let cases=[
        ("pub susp fnc main(): int { val p=task{return 42;}; ^ p |> susp |f| -> await f >> val x: string; }","E_TYPE"),
        ("pub fnc main(): int { val p=task{return 42;}; ^ p |> susp |f| -> await f >> val x: int; }","E_EFFECT"),
        ("pub susp fnc main(): int { val p=task{return 42;}; ^ p |> susp |f| -> await f >> const x: int; }","E_CONST_EFFECT"),
        ("pub susp fnc main(): int { val p=task{return 42;}; ^ p |> susp |f| -> await g >> val x: int; }","E_PIPE_BIND"),
        ("pub susp fnc main(): int { val p=task{return 42;}; ^ p >> val x: int; }","E_IO_SYNTAX"),
    ];
    for (src,expected) in cases {
        let result=compile(src).expect_err("invalid caret pipeline must not emit guest Wasm");
        assert_eq!(result.code,expected,"{src}: {result}");
    }
}

#[test]
fn caret_capture_fails_for_unknown_transformation_and_mismatched_return() {
    for src in [
        "pub susp fnc main(): int { val p=task{return 41;}; ^ p |> susp |f| -> await f |> unknown_transform >> val x: int; }",
        "fnc bool_result(int v): bool { return true; } pub susp fnc main(): int { val p=task{return 41;}; ^ p |> susp |f| -> await f |> bool_result >> val x: bool; }",
    ] {
        assert!(compile(src).is_err(),"source cannot pass without callable or matching return type");
    }
}


#[test]
fn live_class_loans_cannot_cross_suspending_future_pipeline() {
    for loan in ["borrow! parcel", "borrow! mut parcel"] {
        let source = format!(r#"
define class Parcel as
    pub let int value = 1;
end
pub suspends fnc main(): int {{
    let parcel = new Parcel();
    val reader = {loan};
    val future = task {{ return 41; }};
    future |> susp |f| -> await f >> val received: int;
    return received + reader.value;
}}
"#);
        let diag=compile(&source).expect_err("class loan must not cross suspension");
        assert_eq!(diag.code,"E_BORROW","{loan}: {diag}");
    }
}

#[test]
fn completed_class_loan_scope_may_precede_suspending_pipeline() {
    let source = r#"
define class Parcel as
    pub let int value = 1;
end
pub suspends fnc main(): int {
    let parcel = new Parcel();
    block {
        val reader = borrow! parcel;
        stdio.println(reader.value);
    }
    val future = task { return 42; };
    future |> susp |f| -> await f >> val value: int;
    return value;
}
"#;
    compile_ok(source);
}


#[test]
fn optional_directional_equals_emits_identical_executable_wasm() {
    // '=' is a spelling-only alias, not an extra assignment or a new
    // observable evaluation. The emitted checked Future await is identical.
    let canonical = r#"
pub suspends fnc main(): int {
    val pending = task { return 41; };
    pending |> susp |f| -> await f >> val result: int;
    return result + 1;
}
"#;
    let alternate = canonical.replace(">> val result: int", ">> = val result: int");
    assert_eq!(compile_ok(canonical), compile_ok(&alternate));
}

#[test]
fn optional_equals_also_works_in_return_and_caret_terminal_capture() {
    for keyword in ["return", "^"] {
        let src = format!(r#"
pub suspends fnc main(): int {{
    val pending = task {{ return 42; }};
    {keyword} pending |> susp |f| -> await f >> = val result: int;
}}
"#);
        compile_ok(&src);
    }
}

#[test]
fn optional_equals_in_executable_capture_rejects_contiguous_or_repeated_markers() {
    for suffix in [
        ">>= val result: int",
        ">> == val result: int",
        ">> = = val result: int",
        ">> = result",
        "> > = val result: int",
    ] {
        let src = format!(
            "pub suspends fnc main(): int {{ val p=task {{ return 42; }}; \
             p |> susp |f| -> await f {suffix}; return 42; }}"
        );
        compile(&src).expect_err("invalid directional marker must not be executable");
    }

    let contiguous = "pub suspends fnc main(): int { val p=task {return 42;}; p |> susp |f| -> await f >>= val result: int; return 42; }";
    let diag = compile(contiguous).expect_err(">>= must not silently alias >> =");
    assert_eq!(diag.code, "E_IO_SYNTAX", "{diag}");

    let collision = "pub suspends fnc main(): int { val p=task {return 42;}; val result=0; p |> susp |f| -> await f >> = val result:int; return 42; }";
    compile(collision).expect_err("optional marker must not permit redeclaration");
}

#[test]
fn an_equals_cannot_turn_plain_expressions_into_directional_capture() {
    for src in [
        "pub fnc main(): int { 42 >> = val answer: int; return answer; }",
        "pub fnc main(): int { val answer = 41; answer >> = answer; return answer; }",
    ] {
        compile(src).expect_err("directional '=' is not an arithmetic or assignment operator");
    }
}
