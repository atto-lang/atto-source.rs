//! Fail-closed tests for direct lambda desugaring and public AST entry point.
use atto_compiler::{
    ast::{Expr, ExprKind, Func, Stmt, Type, UnOp},
    check, lexer, parser::Parser, Pos,
};

fn source_program(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap()
}
fn simple() -> atto_compiler::ast::Program {
    source_program("pub fnc main(): int { val double = |n| : int -> n * 2; return double(21); }")
}
#[test]
fn erased_lambda_binding_cannot_hide_prior_duplicate() {
    let source = "pub fnc main(): int { val double = 7; val double = |n| -> n * 2; return double(21); }";
    let d = atto_compiler::compile(source).unwrap_err();
    assert_eq!(d.code, "E_DUPLICATE", "{d}");
}
#[test]
fn prior_nested_block_binding_is_not_same_scope_duplicate() {
    let source = "pub fnc main(): int { block { val double = 7; } val double = |n| -> n * 2; return double(21); }";
    assert!(atto_compiler::compile(source).is_ok());
}
#[test]
fn later_same_scope_and_nested_shadowing_reject() {
    for source in [
        "pub fnc main(): int { val double = |n| -> n * 2; val double = 0; return double(21); }",
        "pub fnc main(): int { val double = |n| -> n * 2; block { val double = 0; } return double(21); }",
    ] {
        let d = atto_compiler::compile(source).unwrap_err();
        assert!(matches!(d.code, "E_UNSUPPORTED" | "E_DUPLICATE"), "{d}");
    }
}
#[test]
fn forged_ast_private_function_name_rejects_collision() {
    let mut program = simple();
    let pos = Pos { line: 1, col: 1 };
    program.funcs.push(Func {
        name: "$lambda_0".to_string(), pos, is_generator: false, is_pure: false,
        params: vec![], ret: Type::Int,
        body: vec![Stmt::Return(Some(Expr {
            kind: ExprKind::Int(Some(0)), pos,
        }), pos)],
    });
    let d = check::check(&program).unwrap_err();
    assert_eq!(d.code, "E_DUPLICATE", "{d}");
}
#[test]
fn nested_statement_ast_rewriter_respects_depth_limit() {
    let mut program = simple();
    let mut statement = Stmt::Block(Vec::new());
    for _ in 0..140 { statement = Stmt::Block(vec![statement]); }
    program.funcs[0].body.insert(1, statement);
    let d = check::check(&program).unwrap_err();
    assert_eq!(d.code, "E_NESTING", "{d}");
}
#[test]
fn nested_expression_ast_rewriter_respects_depth_limit() {
    let mut program = simple();
    let pos = Pos { line: 1, col: 1 };
    let mut inner = Expr { kind: ExprKind::Int(Some(1)), pos };
    for _ in 0..140 {
        inner = Expr { kind: ExprKind::Unary(UnOp::Plus, Box::new(inner)), pos };
    }
    let Stmt::Return(Some(e), _) = &mut program.funcs[0].body[1] else {
        panic!("expected direct return")
    };
    *e = inner;
    let d = check::check(&program).unwrap_err();
    assert_eq!(d.code, "E_NESTING", "{d}");
}
#[test]
fn direct_lambda_name_collision_does_not_affect_unrelated_functions() {
    let source = "pub fnc triple(int n): int { return n * 3; } pub fnc main(): int { val double = |n| -> n * 2; return double(21); }";
    let p = source_program(source);
    let typed = check::check(&p).expect("direct lambda with extra top-level function");
    assert_eq!(typed.funcs.len(), 3);
    assert_eq!(typed.funcs[2].name, "$lambda_1");
    assert!(atto_compiler::compile(source).is_ok());
}


#[test]
fn lambda_calls_in_constructor_arguments_bind_to_definition_site() {
    // The top-level function deliberately has the same name as the local
    // direct lambda. Missing New(arg) recursion otherwise resolves this call
    // to the wrong function and silently changes the result.
    let src = r#"
fnc transform(int n): int { return n * 100; }
define class Container as
  mut int value = 0;
  constructor(int v) { self.value = v; }
  pub get(): int { return self.value; }
end
pub fnc main(): int {
  val factor = 2;
  val transform = |n| : int -> n * factor;
  block {
    val factor = 100;
    val container = new Container(transform(21));
    return container.get();
  }
}
"#;
    let p = source_program(src);
    let checked = check::check(&p).expect("constructor call must be rewritten");
    assert!(checked.funcs.iter().any(|f| f.name == "$lambda_2"));
    assert!(atto_compiler::compile(src).is_ok());
}

#[test]
fn constructor_argument_cannot_escape_direct_lambda_as_value() {
    let src = r#"
define class Holder as
  mut int value = 0;
  constructor(int v) { self.value = v; }
end
pub fnc main(): int {
  val direct = |n| : int -> n + 1;
  val holder = new Holder(direct);
  return direct(41);
}
"#;
    let error = atto_compiler::compile(src).unwrap_err();
    assert_eq!(error.code, "E_UNSUPPORTED", "{error}");
}
