//! First executable slice: typed private Wasm direct functions, no captures.
use atto_compiler::{
    ast::{ExprKind, Type}, check::{self, TExpr, TStmt}, lexer, parser::Parser
};

fn checked(source: &str) -> check::TProgram {
    let p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    check::check(&p).unwrap()
}
#[test]
fn direct_lambda_generates_typed_private_function() {
    let p = checked("pub fnc main(): int { val double = |n| : int -> n * 2; return double(21); }");
    assert_eq!(p.funcs.len(), 2);
    assert_eq!(p.funcs[1].name, "$lambda_0");
    assert_eq!(p.funcs[1].params, [Type::Int]);
    assert_eq!(p.funcs[1].ret, Type::Int);
    let TStmt::Return(Some(value)) = &p.funcs[0].body[0] else {
        panic!("expected direct call return")
    };
    assert!(matches!(&value.e, TExpr::Call(1, args) if args.len() == 1));
}
#[test]
fn explicit_statement_return_compiles_to_typed_function() {
    let source = "pub fnc main(): int { val double = (|n| : int -> return n * 2;); return double(21); }";
    let p = checked(source);
    assert_eq!(p.funcs[1].ret, Type::Int);
    assert!(atto_compiler::compile(source).is_ok());
}
#[test]
fn zero_parameter_lambda_and_deterministic_wasm() {
    let src = "pub fnc main(): int { val answer = || -> 42; return answer(); }";
    let wasm = atto_compiler::compile(src).unwrap();
    assert_eq!(atto_compiler::compile(src).unwrap(), wasm);
    assert!(wasm.starts_with(b"\0asm"));
}
#[test]
fn immutable_scalar_capture_is_lifted_as_hidden_wasm_parameter() {
    let src = "pub fnc main(): int { val extra = 1; val f = |n| -> n + extra; return f(1); }";
    let p = checked(src);
    assert_eq!(p.funcs[1].params, [Type::Int, Type::Int]);
    assert!(atto_compiler::compile(src).is_ok());
    let TStmt::Return(Some(value)) = &p.funcs[0].body[2] else {
        panic!("expected direct captured call return")
    };
    assert!(matches!(&value.e, TExpr::Call(1, args) if args.len() == 2));
}
#[test]
fn mutable_outer_capture_still_rejected() {
    let src = "pub fnc main(): int { let extra = 1; val f = |n| -> n + extra; return f(1); }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_CAPTURE");
}
#[test]
fn lambda_escape_does_not_silently_create_callable() {
    let src = "pub fnc main(): int { val f = |n| -> n + 1; val alias = f; return f(1); }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn not_called_lambda_remains_unsupported() {
    let src = "pub fnc main(): int { val f = |n| -> n + 1; return 0; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}
