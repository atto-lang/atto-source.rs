//! Lambda parser acceptance and fail-closed backend contract.
use atto_compiler::{ast::{ExprKind, LambdaBody, Stmt}, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap()
}

fn main_with(expr: &str) -> String {
    format!("pub fnc main(): int {{ val values = make_values(); val result = {expr}; return 0; }}")
}

#[test]
fn colon_annotated_rx_pipeline_and_arrays_parse() {
    let source = r#"
        pub fnc main(): int {
            val values = rx.Observable.from_iter([1, 2, 3, 4, 5]);
            val small_doubled = values
                .filter(|n| : bool -> return n < 4;)
                .map(|n| : int -> return n * 2;)
                .take(2);
            return 0;
        }
    "#;
    let p = parse(source);
    let Stmt::Bind { init, .. } = &p.funcs[0].body[1] else { panic!("expected binding") };
    let ExprKind::MethodCall(base, method, _) = &init.kind else { panic!("expected take") };
    assert_eq!(method, "take");
    let ExprKind::MethodCall(base, method, args) = &base.kind else { panic!("expected map") };
    assert_eq!(method, "map");
    let ExprKind::Lambda { params, return_type, body } = &args[0].kind else { panic!("expected lambda") };
    assert_eq!(params, &["n"]);
    assert_eq!(return_type.as_deref(), Some("int"));
    assert!(matches!(body, LambdaBody::Statement(_)));
    let ExprKind::MethodCall(_, method, args) = &base.kind else { panic!("expected filter") };
    assert_eq!(method, "filter");
    let ExprKind::Lambda { return_type, .. } = &args[0].kind else { panic!("expected lambda") };
    assert_eq!(return_type.as_deref(), Some("bool"));
}

#[test]
fn inferred_expression_and_statement_bodies_parse() {
    for expr in [
        "values.map(|n| -> n * 2)",
        "values.map(|n| -> return n * 2;)",
        "values.map(|n| : int -> n * 2)",
        "values.map(|n| : int -> return n * 2;)",
        "values.map(|n| -> int { return n * 2; })",
        "values.map(|n| => int -> return n * 2;)",
        "values.map(|n| : Result<int,Error> -> n)",
        "values.map(|n| : my.types.Response -> n)",
        "values.map(|n| -> |m| -> n + m)",
        "values.map(|| -> 1)",
    ] {
        let p = parse(&main_with(expr));
        assert_eq!(p.funcs[0].body.len(), 3, "{expr}");
    }
}

#[test]
fn lambda_parser_rejects_bad_signatures_and_unterminated_return() {
    for (expr, code) in [
        ("values.map(|n| : -> return n;)", "E_SYNTAX"),
        ("values.map(|n,n| -> n)", "E_DUPLICATE"),
        ("values.map(|n| : int return n;)", "E_SYNTAX"),
        ("values.map(|n| : int -> return n)", "E_SYNTAX"),
        ("values.map(|n| : int => int -> return n;)", "E_SYNTAX"),
        ("values.map(|n| -> val x = n; return x;)", "E_SYNTAX"),
    ] {
        let src = main_with(expr);
        let d = Parser::new(lexer::lex(&src).unwrap()).parse_program().unwrap_err();
        assert_eq!(d.code, code, "{expr}: {d}");
    }
}

#[test]
fn parsed_but_unlowered_lambda_fails_closed() {
    let src = "pub fnc main(): int { val mapper = |n| : int -> n * 2; return 0; }";
    let err = atto_compiler::compile(src).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("lambdas"), "{err}");
}
