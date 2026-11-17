//! Contextual noncapturing lambda type checking; no Wasm closure emission.
use atto_compiler::{
    ast::{Expr, ExprKind, Stmt, Type},
    check::lambda_types::check_noncapturing, lexer, parser::Parser,
};

fn callback(src: &str) -> Expr {
    let input = format!("pub fnc main(): int {{ val callback = {src}; return 0; }}");
    let ast = Parser::new(lexer::lex(&input).unwrap()).parse_program()
        .unwrap_or_else(|d| panic!("{src}: {d}"));
    let Stmt::Bind { init, .. } = &ast.funcs[0].body[0] else { unreachable!() };
    assert!(matches!(init.kind, ExprKind::Lambda { .. }));
    init.clone()
}

fn check(src: &str, params: Option<&[Type]>, result: Option<Type>) ->
    Result<Type, atto_compiler::Diagnostic> {
    check_noncapturing(&callback(src), params, result).map(|c| c.result)
}

#[test]
fn contextual_filter_and_map_types_infer() {
    let params = &[Type::Int][..];
    assert_eq!(check("|n| : bool -> return n < 4;", Some(params), Some(Type::Bool)), Ok(Type::Bool));
    assert_eq!(check("|n| -> n < 4", Some(params), None), Ok(Type::Bool));
    assert_eq!(check("|n| : int -> return n * 2;", Some(params), Some(Type::Int)), Ok(Type::Int));
    assert_eq!(check("|n| -> n * 2", Some(params), None), Ok(Type::Int));
    assert_eq!(check("|n| => int -> return n * 2;", Some(params), None), Ok(Type::Int));
    let sig = check_noncapturing(&callback("|n| : bool -> n < 4"), Some(params), None).unwrap();
    assert_eq!(sig.params, [("n".to_string(), Type::Int)]);
    assert_eq!(sig.result, Type::Bool);
    assert!(sig.annotated);
}

#[test]
fn inferred_params_require_expected_callable_signature() {
    assert_eq!(check("|n| -> n + 1", None, None).unwrap_err().code, "E_INFER");
    assert_eq!(check("|n| -> n + 1", Some(&[]), None).unwrap_err().code, "E_ARITY");
    assert_eq!(check("|n| -> n + 1", Some(&[Type::Void]), None).unwrap_err().code, "E_UNSUPPORTED_TYPE");
    assert_eq!(check("|| -> 42", None, None), Ok(Type::Int));
    assert_eq!(check("|| : bool -> true", Some(&[]), Some(Type::Bool)), Ok(Type::Bool));
    assert_eq!(check("|a,b| -> a + b", Some(&[Type::Int, Type::Int]), None), Ok(Type::Int));
}

#[test]
fn return_annotations_and_all_paths_are_checked() {
    let p = Some(&[Type::Int][..]);
    assert_eq!(check("|n| : bool -> n * 2", p, None).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| -> n * 2", p, Some(Type::Bool)).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| : bool -> n < 4", p, Some(Type::Int)).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| -> int { return n; }", p, None), Ok(Type::Int));
    assert_eq!(check("|n| -> { if n < 4 then return n; else return n * 2; fi }", p, None), Ok(Type::Int));
    assert_eq!(check("|n| -> { if n < 4 then return n; fi }", p, None).unwrap_err().code, "E_RETURN");
    assert_eq!(check("|n| -> { if n < 4 then return n; else return true; fi }", p, None).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| : int -> { val doubled = n * 2; return doubled; }", p, None), Ok(Type::Int));
    assert_eq!(check("|n| : void -> { val doubled = n * 2; }", p, None), Ok(Type::Void));
    assert_eq!(check("|n| : int -> { val doubled = n * 2; }", p, None).unwrap_err().code, "E_RETURN");
}

#[test]
fn captures_unsafe_effects_and_assignments_fail_closed() {
    let p = Some(&[Type::Int][..]);
    for src in [
        "|n| -> n + outer",
        "|n| -> self",
        "|n| -> stdio.print(n)",
        "|n| -> n.foo",
        "|n| -> |m| -> n + m",
    ] {
        let code = check(src, p, None).unwrap_err().code;
        assert!(matches!(code, "E_CAPTURE" | "E_UNSUPPORTED"), "{src}: {code}");
    }
    assert_eq!(check("|n| -> { val x = n; x = 3; return x; }", p, None).unwrap_err().code, "E_IMMUTABLE");
    assert_eq!(check("|n| -> { let x = n; x = 3; return x; }", p, None), Ok(Type::Int));
    assert_eq!(check("|n| -> { let x = n; x = true; return x; }", p, None).unwrap_err().code, "E_TYPE");
}

#[test]
fn scalar_operators_have_native_checker_semantics() {
    assert_eq!(check("|n| -> n / 2", Some(&[Type::Int]), None).unwrap_err().code, "E_UNSUPPORTED");
    assert_eq!(check("|n| -> n + 0.5", Some(&[Type::Int]), None), Ok(Type::Float));
    assert_eq!(check("|n| -> n < 2.5", Some(&[Type::Float]), None), Ok(Type::Bool));
    assert_eq!(check("|n| -> n + true", Some(&[Type::Int]), None).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| -> n eq 4", Some(&[Type::Int]), None), Ok(Type::Bool));
    assert_eq!(check("|n| -> n eq true", Some(&[Type::Int]), None).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| -> n == 4", Some(&[Type::Int]), None).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| -> n != 4", Some(&[Type::Int]), None).unwrap_err().code, "E_TYPE");
    assert_eq!(check("|n| -> n + \"z\"", Some(&[Type::Str]), None), Ok(Type::Str));
}

#[test]
fn analyzer_success_does_not_enable_wasm_execution() {
    let code = "pub fnc main(): int { val callback = (|n| : int -> n * 2); return 0; }";
    let d = atto_compiler::compile(code).unwrap_err();
    assert_eq!(d.code, "E_UNSUPPORTED");
    assert!(d.message.contains("lambdas"));
}
