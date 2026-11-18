//! Security and compatibility hardening for lambda parsing.
//! These test syntax only; the browser compiler must reject execution until
//! callable types, ownership-aware captures and Wasm lowering exist.

use atto_compiler::{
    ast::{BinOp, ExprKind, LambdaBody, Stmt},
    lexer,
    parser::{Parser, MAX_PARAMS},
};

fn program(expr: &str) -> String {
    format!("pub fnc main(): int {{ val callback = {expr}; return 0; }}")
}

fn parsed(expr: &str) -> Result<atto_compiler::ast::Program, atto_compiler::Diagnostic> {
    Parser::new(lexer::lex(&program(expr))?).parse_program()
}

fn lambda(expr: &str) -> (Vec<String>, Option<String>) {
    let p = parsed(expr).unwrap_or_else(|e| panic!("{expr}: {e}"));
    let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else {
        panic!("not binding")
    };
    let ExprKind::Lambda { params, return_type, .. } = &init.kind else {
        panic!("not lambda")
    };
    (params.clone(), return_type.clone())
}

#[test]
fn qualified_and_nested_generic_return_types() {
    for (input, expected) in [
        ("|n| : my.types.Response -> n", "my.types.Response"),
        ("|n| : a.module.Type<Result<int,Error>> -> n", "a.module.Type<Result<int,Error>>"),
        ("|n| : Foo<Bar<Baz>> -> n", "Foo<Bar<Baz>>"),
        ("|n| -> my.types.Response { return n; }", "my.types.Response"),
        ("|n| : Result<int,Error> -> return n;", "Result<int,Error>"),
    ] {
        let (params, ty) = lambda(input);
        assert_eq!(params, ["n"], "{input}");
        assert_eq!(ty.as_deref(), Some(expected), "{input}");
    }
    // Keywords are legal only in unambiguous *qualified* segments, never in
    // a parameter position or as a standalone return-type head.
    assert!(parsed("|if| : bool -> true").is_err());
    assert!(parsed("|n| : if -> true").is_err());
    assert!(parsed("|n| : foo. -> true").is_err());
}

#[test]
fn legacy_lookahead_does_not_turn_expression_into_type() {
    for (expr, want) in [
        ("|n| -> n * 2", None),
        ("|n| -> foo.bar + 2", None),
        ("|n| -> Result<int,Error> { return n; }", Some("Result<int,Error>")),
        ("|n| -> n.value", None),
        ("|n| -> foo(n)", None),
        ("|n| -> |m| : int -> n + m", None),
    ] {
        let (_, annotation) = lambda(expr);
        assert_eq!(annotation.as_deref(), want, "{expr}");
    }
}

#[test]
fn lambda_parameters_respect_limit_and_uniqueness() {
    let good = (0..MAX_PARAMS).map(|i| format!("p{i}")).collect::<Vec<_>>().join(",");
    let bad = (0..=MAX_PARAMS).map(|i| format!("p{i}")).collect::<Vec<_>>().join(",");
    assert_eq!(lambda(&format!("|{good}| -> 1")).0.len(), MAX_PARAMS);
    assert_eq!(parsed(&format!("|{bad}| -> 1")).unwrap_err().code, "E_SOURCE_LIMIT");
    assert_eq!(parsed("|a,b,a| -> a").unwrap_err().code, "E_DUPLICATE");
}

#[test]
fn nesting_limits_are_not_laundered_by_legacy_type_lookahead() {
    let mut deep_type = "int".to_string();
    for _ in 0..150 {
        deep_type = format!("Box<{deep_type}>");
    }
    let explicit = format!("|n| : {deep_type} -> n");
    let legacy = format!("|n| -> {deep_type} {{ return n; }}");
    assert_eq!(parsed(&explicit).unwrap_err().code, "E_NESTING");
    assert_eq!(parsed(&legacy).unwrap_err().code, "E_NESTING");
    let nested = format!("{}1", "|x| -> ".repeat(150));
    assert_eq!(parsed(&nested).unwrap_err().code, "E_NESTING");
}

#[test]
fn inner_return_semicolon_cannot_be_omitted_or_reused() {
    for expression in [
        "values.map(|n| -> return n)",
        "values.map(|n| -> return n\n)",
        "values.map(|n| : int -> return n, other)",
        "values.map(|n| : int -> return n\n, other)",
    ] {
        let program = format!("pub fnc main(): int {{ val x = {expression}; return 0; }}");
        let err = Parser::new(lexer::lex(&program).unwrap()).parse_program().unwrap_err();
        assert_eq!(err.code, "E_SEMICOLON", "{expression}: {err}");
    }
    for expression in [
        "values.map(|n| : int -> return n;, 9)",
        "values.map(|n| -> n, 9)",
        "values.map(|n| -> return n;).take(2)",
    ] {
        let program = format!("pub fnc main(): int {{ val x = {expression}; return 0; }}");
        Parser::new(lexer::lex(&program).unwrap())
            .parse_program()
            .unwrap_or_else(|e| panic!("{expression}: {e}"));
    }
}

#[test]
fn conditional_or_still_parses_as_or_and_not_as_lambda() {
    let p = Parser::new(lexer::lex("pub fnc main(): int { if true | false then return 1; fi return 0; }").unwrap())
        .parse_program()
        .unwrap();
    let Stmt::If { arms, .. } = &p.funcs[0].body[0] else {
        panic!("expected if")
    };
    assert!(matches!(arms[0].0.kind, ExprKind::Binary(BinOp::Or, _, _)));
}

#[test]
fn unsupported_callable_and_array_execution_fails_closed() {
    for source in [
        "pub fnc main(): int { val f = |n| : int -> n * 2; return 0; }",
        "pub fnc main(): int { val f = (|n| -> return n;); return 0; }",
        "pub fnc main(): int { val xs = [1,2,3]; return 0; }",
    ] {
        let err = atto_compiler::compile(source).expect_err("must never emit Wasm");
        assert_eq!(err.code, "E_UNSUPPORTED", "{source}: {err}");
    }
}

#[test]
fn braced_and_expression_forms_preserve_body_kinds() {
    for (source, expected) in [
        ("|n| : int -> n * 2", 0),
        ("|n| : int -> return n * 2;", 1),
        ("|n| -> int { val x = n * 2; return x; }", 2),
        ("|n| => int -> return n * 2;", 1),
    ] {
        let p = parsed(source).unwrap();
        let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else {
            unreachable!()
        };
        let ExprKind::Lambda { body, .. } = &init.kind else {
            unreachable!()
        };
        let actual = match body {
            LambdaBody::Expression(_) => 0,
            LambdaBody::Statement(_) => 1,
            LambdaBody::Block(_) => 2,
        };
        assert_eq!(actual, expected, "{source}");
    }
}
