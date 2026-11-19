//! Typed callable metadata is validated but does not enable first-class
//! source function types or Wasm indirect calls.
use atto_compiler::{
    ast::{Expr, ExprKind, Stmt, Type},
    check::{self, callable_ir::{CallableSignature, CallableTarget, TypedCallable}},
    lexer, parser::Parser, Pos,
};

fn parse_lambda(text: &str) -> Expr {
    let source = format!("pub fnc main(): int {{ val callback = {text}; return 0; }}");
    let program = Parser::new(lexer::lex(&source).expect("lex"))
        .parse_program().expect("lambda parsed");
    let Stmt::Bind { init, .. } = &program.funcs[0].body[0] else {
        panic!("expected callback binding")
    };
    assert!(matches!(init.kind, ExprKind::Lambda { .. }));
    init.clone()
}

#[test]
fn scalar_signature_supports_checked_contextual_lambda() {
    let lambda = parse_lambda("|n| : bool -> n < 4");
    let sig = CallableSignature::from_noncapturing(
        &lambda, Some(&[Type::Int]), Some(Type::Bool)).unwrap();
    assert_eq!(sig.params, [Type::Int]);
    assert_eq!(sig.result, Type::Bool);
    assert_eq!(sig.check_arguments(&[Type::Int], lambda.pos).unwrap(), Type::Bool);
}

#[test]
fn invocation_checks_arity_and_exact_types() {
    let p = Pos { line: 1, col: 1 };
    let sig = CallableSignature::scalar(vec![Type::Int, Type::Float], Type::Bool, p).unwrap();
    assert_eq!(sig.check_arguments(&[Type::Int], p).unwrap_err().code, "E_ARITY");
    assert_eq!(sig.check_arguments(&[Type::Int, Type::Int], p).unwrap_err().code, "E_TYPE");
    assert_eq!(sig.check_arguments(&[Type::Int, Type::Float], p).unwrap(), Type::Bool);
}

#[test]
fn scalar_signature_rejects_unsendable_or_unlowered_types() {
    let p = Pos { line: 1, col: 1 };
    assert_eq!(CallableSignature::scalar(vec![Type::Void], Type::Int, p).unwrap_err().code,
        "E_UNSUPPORTED_TYPE");
    assert_eq!(CallableSignature::scalar(vec![Type::Class(0)], Type::Int, p).unwrap_err().code,
        "E_UNSUPPORTED_TYPE");
    assert_eq!(CallableSignature::scalar(vec![Type::Int], Type::GeneratorInt, p).unwrap_err().code,
        "E_UNSUPPORTED_TYPE");
    assert_eq!(CallableSignature::scalar(vec![Type::Int; 33], Type::Int, p).unwrap_err().code,
        "E_SOURCE_LIMIT");
    assert!(CallableSignature::scalar(vec![], Type::Void, p).is_ok());
}

#[test]
fn wrong_contextual_annotation_and_captures_stay_rejected() {
    let lambda = parse_lambda("|n| : int -> n < 4");
    assert_eq!(CallableSignature::from_noncapturing(&lambda, Some(&[Type::Int]), None)
        .unwrap_err().code, "E_TYPE");
    let capture = parse_lambda("|n| -> n + outside");
    assert_eq!(CallableSignature::from_noncapturing(&capture, Some(&[Type::Int]), None)
        .unwrap_err().code, "E_CAPTURE");
}

#[test]
fn direct_target_is_index_bounded() {
    let p = Pos { line: 1, col: 1 };
    let sig = CallableSignature::scalar(vec![Type::Int], Type::Int, p).unwrap();
    let c = TypedCallable::checked(sig.clone(), CallableTarget::Direct { function_index: 1 }, p)
        .unwrap();
    assert_eq!(c.target(), CallableTarget::Direct { function_index: 1 });
    assert_eq!(c.signature(), &sig);
    assert_eq!(c.check_arguments(&[Type::Int], p).unwrap(), Type::Int);
    assert_eq!(TypedCallable::checked(sig, CallableTarget::Direct { function_index: 1024 }, p)
        .unwrap_err().code, "E_SOURCE_LIMIT");
}

#[test]
fn indirect_target_is_not_prematurely_executable() {
    let p = Pos { line: 1, col: 1 };
    let sig = CallableSignature::scalar(vec![Type::Int], Type::Int, p).unwrap();
    let intent = CallableTarget::Indirect {
        table_index: 1, generation: 7, type_index: 2
    };
    let err = TypedCallable::checked(sig, intent, p).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
}

#[test]
fn integration_keeps_same_direct_lambda_function_indices() {
    let source = "pub fnc main(): int { val double = |n| : int -> n * 2; return double(21); }";
    let ast = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let typed = check::check(&ast).unwrap();
    assert_eq!(typed.funcs.len(), 2);
    assert_eq!(typed.funcs[1].name, "$lambda_0");
    assert_eq!(typed.funcs[1].params, [Type::Int]);
    let wasm = atto_compiler::compile(source).expect("direct lambda stays executable");
    assert_eq!(wasm, atto_compiler::compile(source).unwrap());
}

#[test]
fn passing_callable_as_value_remains_fail_closed() {
    let source = "pub fnc main(): int { val double = |n| -> n * 2; val alias = double; return 0; }";
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}
