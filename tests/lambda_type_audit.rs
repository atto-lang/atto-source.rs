//! Regression audit for stand-alone contextual lambda analyzer.
//! No callable closure or Wasm artifact may be inferred from this check.
use atto_compiler::{
    ast::{Expr, ExprKind, LambdaBody, Stmt, Type},
    check::lambda_types::check_noncapturing,
    diag::Pos, lexer, parser::Parser,
};

fn parsed_lambda(input: &str) -> Expr {
    let source = format!("pub fnc main(): int {{ val callback = {input}; return 0; }}");
    let p = Parser::new(lexer::lex(&source).expect("lex"))
        .parse_program().unwrap_or_else(|e| panic!("{input}: {e}"));
    let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else { panic!("lambda binding") };
    init.clone()
}
fn checked(input: &str) -> Result<Type, atto_compiler::diag::Diagnostic> {
    check_noncapturing(&parsed_lambda(input), Some(&[Type::Int]), None).map(|c| c.result)
}

#[test]
fn rejects_unreachable_return_of_incompatible_type() {
    let d = checked("|n| -> { return n; return true; }").unwrap_err();
    assert_eq!(d.code, "E_UNREACHABLE", "{d}");
}

#[test]
fn rejects_unreachable_capture_and_effectful_calls() {
    for expression in [
        "|n| -> { return n; val leak = outer; }",
        "|n| -> { return n; stdio.println(n); }",
        "|n| -> { return n; n.foo(); }",
    ] {
        let d = checked(expression).unwrap_err();
        assert_eq!(d.code, "E_UNREACHABLE", "{expression}: {d}");
    }
}

#[test]
fn rejects_code_after_all_branches_return() {
    let d = checked("|n| -> { if n < 3 then return n; else return n * 2; fi return true; }")
        .unwrap_err();
    assert_eq!(d.code, "E_UNREACHABLE", "{d}");
}

#[test]
fn valid_mixed_branch_return_then_fallthrough() {
    assert_eq!(checked("|n| -> { if n < 3 then return n; fi return n * 2; }"),
        Ok(Type::Int));
}

#[test]
fn nested_scope_and_local_shadowing_stay_lexical() {
    assert_eq!(checked("|n| -> { val x = n; block { val x = 2; } return x; }"),
        Ok(Type::Int));
    let d = checked("|n| -> { block { val x = n; } return x; }").unwrap_err();
    assert_eq!(d.code, "E_CAPTURE", "{d}");
}

#[test]
fn direct_ast_cannot_rebind_self() {
    let p = Pos::default();
    let e = Expr {
        kind: ExprKind::Lambda {
            params: vec!["self".into()], param_types: vec![None], return_type: None,
            body: LambdaBody::Expression(Box::new(Expr { kind: ExprKind::Int(Some(1)), pos: p })),
        },
        pos: p,
    };
    let d = check_noncapturing(&e, Some(&[Type::Int]), None).unwrap_err();
    assert_eq!(d.code, "E_RESERVED", "{d}");
}

#[test]
fn direct_ast_nested_statement_budget_fails_closed() {
    let p = Pos::default();
    let mut stmt = Stmt::Return(Some(Expr { kind: ExprKind::Int(Some(1)), pos: p }), p);
    for _ in 0..140 {
        stmt = Stmt::Block(vec![stmt]);
    }
    let e = Expr {
        kind: ExprKind::Lambda {
            params: vec![], param_types: vec![], return_type: Some("int".into()),
            body: LambdaBody::Block(vec![stmt]),
        },
        pos: p,
    };
    let d = check_noncapturing(&e, Some(&[]), Some(Type::Int)).unwrap_err();
    assert_eq!(d.code, "E_NESTING", "{d}");
}

#[test]
fn direct_ast_deep_expressions_keep_limit() {
    let p = Pos::default();
    let mut value = Expr { kind: ExprKind::Int(Some(1)), pos: p };
    for _ in 0..140 {
        value = Expr { kind: ExprKind::Unary(atto_compiler::ast::UnOp::Plus, Box::new(value)), pos: p };
    }
    let e = Expr {
        kind: ExprKind::Lambda { params: vec![], param_types: vec![], return_type: None,
            body: LambdaBody::Expression(Box::new(value)) }, pos: p,
    };
    let d = check_noncapturing(&e, None, None).unwrap_err();
    assert_eq!(d.code, "E_NESTING", "{d}");
}

#[test]
fn standalone_checker_still_does_not_emit_wasm() {
    let source = "pub fnc main(): int { val callback = (|n| : int -> n + 1); return 0; }";
    let d = atto_compiler::compile(source).unwrap_err();
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
}
