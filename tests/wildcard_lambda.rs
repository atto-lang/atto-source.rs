//! Standalone '_' is an arity-polymorphic, fully-discarding lambda.
//! These tests cover AST identity, contextual typing, and real Rust/Wasm lowering.
use atto_compiler::{
    ast::{Expr, ExprKind, Stmt, Type},
    check::{self, lambda_types::check_noncapturing},
    lexer, parser::{Parser, MAX_PARAMS},
};

fn parse_lambda(source: &str) -> Expr {
    let source = format!("pub fnc main(): int {{ val callback = {source}; return 0; }}");
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let Stmt::Bind { init, .. } = &parsed.funcs[0].body[0] else { panic!("binding expected") };
    init.clone()
}
fn compile(source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    atto_compiler::compile(source)
}

#[test]
fn standalone_wildcard_is_distinct_from_exact_arity_pipe_lambdas() {
    let wildcard = parse_lambda("_ : int -> 5");
    assert!(matches!(&wildcard.kind,
        ExprKind::Lambda { params, variadic: true, return_type: Some(r), .. }
        if params.is_empty() && r == "int"));
    let zero = parse_lambda("|| : int -> 5");
    assert!(matches!(&zero.kind, ExprKind::Lambda { params, variadic: false, .. } if params.is_empty()));
    let one = parse_lambda("|_| : int -> 5");
    assert!(matches!(&one.kind, ExprKind::Lambda { params, variadic: false, .. } if params == &["_"]));
    let two = parse_lambda("|_, _| : int -> 5");
    assert!(matches!(&two.kind, ExprKind::Lambda { params, variadic: false, .. } if params == &["_", "_"]));
    let untyped = parse_lambda("_ -> 5");
    assert!(matches!(&untyped.kind, ExprKind::Lambda { variadic: true, return_type: None, .. }));
}

#[test]
fn contextual_wildcard_accepts_zero_to_bounded_arity_with_no_bindings() {
    let wildcard = parse_lambda("_ : int -> 5");
    for n in [0, 1, 2, MAX_PARAMS] {
        let types = vec![Type::Int; n];
        let checked = check_noncapturing(&wildcard, Some(&types), Some(Type::Int)).unwrap();
        assert_eq!(checked.result, Type::Int);
        assert_eq!(checked.params.len(), n);
        assert!(checked.params.iter().all(|(name, _)| name.starts_with("$lambda_discard_")));
    }
    assert_eq!(check_noncapturing(&wildcard, None, None).unwrap_err().code, "E_INFER");
    assert_eq!(check_noncapturing(&wildcard, Some(&[Type::Int]), Some(Type::Bool))
        .unwrap_err().code, "E_TYPE");
    assert_eq!(check_noncapturing(&wildcard, Some(&vec![Type::Int; MAX_PARAMS + 1]), None)
        .unwrap_err().code, "E_SOURCE_LIMIT");
    assert_eq!(check_noncapturing(&parse_lambda("_ : int -> outer"),
        Some(&[]), None).unwrap_err().code, "E_CAPTURE");
    // A discarded argument must not be available to the body under '_'.
    assert_eq!(check_noncapturing(&parse_lambda("|_| -> _"),
        Some(&[Type::Int]), None).unwrap_err().code, "E_CAPTURE");
}

#[test]
fn direct_wasm_specializes_distinct_arity_and_type_signatures() {
    let source = "pub fnc main(): int { \
      val constant = _ : int -> 5; \
      val a = constant(); \
      val b = constant(7); \
      val c = constant(true, 9); \
      return a + b + c; \
    }";
    let parsed = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let lowered = check::check(&parsed).unwrap();
    assert_eq!(lowered.funcs.len(), 4);
    assert!(lowered.funcs[1].params.is_empty());
    assert_eq!(lowered.funcs[2].params, [Type::Int]);
    assert_eq!(lowered.funcs[3].params, [Type::Bool, Type::Int]);
    let wasm = compile(source).unwrap();
    assert!(wasm.starts_with(b"\0asm"));
    assert_eq!(compile(source).unwrap(), wasm);
}

#[test]
fn direct_wasm_reuses_specialization_for_equal_signatures() {
    let source = "pub fnc main(): int { val c = _ : int -> 5; return c(1) + c(2); }";
    let parsed = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let lowered = check::check(&parsed).unwrap();
    assert_eq!(lowered.funcs.len(), 2);
    assert_eq!(lowered.funcs[1].params, [Type::Int]);
    assert!(compile(source).is_ok());
}

#[test]
fn explicit_pipe_arity_is_still_strict() {
    assert_eq!(compile("pub fnc main(): int { val c = |_| : int -> 5; return c(); }")
        .unwrap_err().code, "E_ARITY");
    assert!(compile("pub fnc main(): int { val c = |_, _| : int -> 5; return c(1, 2); }").is_ok());
    assert_eq!(compile("pub fnc main(): int { val c = |_, _| : int -> 5; return c(1); }")
        .unwrap_err().code, "E_ARITY");
}

#[test]
fn wildcard_direct_calls_fail_closed_without_concrete_scalar_signatures() {
    assert_eq!(compile("pub fnc main(): int { val c = _ : int -> 5; return c(outer); }")
        .unwrap_err().code, "E_INFER");
    assert_eq!(compile("pub fnc main(): int { val c = _ : int -> 5; return 0; }")
        .unwrap_err().code, "E_UNSUPPORTED");
    // Standalone '_' does not make actors, escaped closures, or runtime variadics executable.
    assert_eq!(compile("pub fnc main(): int { val c = _ : int -> 5; val alias = c; return c(1); }")
        .unwrap_err().code, "E_UNSUPPORTED");
}
