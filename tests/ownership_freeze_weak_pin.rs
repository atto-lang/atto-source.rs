//! The browser/compiler subset must distinguish parsing a rights operation
//! from *proving* its memory safety. An unimplemented operation must never
//! silently lower to an ordinary strong alias or a movable guest pointer.
use atto_compiler::{
    ast::{ExprKind, OwnershipOp, Stmt},
    compile, lexer, parser::Parser,
};

fn first_initializer(source: &str) -> atto_compiler::ast::Expr {
    let program = Parser::new(lexer::lex(source).expect("lex"))
        .parse_program().expect("parse");
    let Stmt::Bind { init, .. } = &program.funcs[0].body[0] else {
        panic!("expected binding")
    };
    init.clone()
}

fn source(expr: &str) -> String {
    format!("pub fnc main(): int {{ val result = {expr}; return 0; }}")
}

#[test]
fn all_three_new_forms_are_explicit_ownership_ast_nodes() {
    for (spelling, expected) in [
        ("freeze! 42", OwnershipOp::Freeze),
        ("weak! 42", OwnershipOp::Weak),
        ("pin! 42", OwnershipOp::Pin),
    ] {
        let ast = first_initializer(&source(spelling));
        assert!(matches!(ast.kind, ExprKind::Ownership(op, false, _) if op == expected),
            "{spelling} was not parsed as an ownership operation");
    }
}

#[test]
fn nested_prefix_ownership_preserves_order_and_source_position() {
    let ast = first_initializer(&source("freeze! take! owner"));
    let ExprKind::Ownership(OwnershipOp::Freeze, false, operand) = ast.kind else {
        panic!("outer freeze! must not disappear");
    };
    assert!(matches!(operand.kind, ExprKind::Ownership(OwnershipOp::Take, false, _)));
    assert!(ast.pos.col > 0);
}

#[test]
fn deeply_immutable_scalar_freeze_is_executable_identity() {
    for expr in ["freeze! 42", "freeze! true", "freeze! \"hello\""] {
        let src = format!("pub fnc main(): int {{ val x = {expr}; return 42; }}");
        let wasm = compile(&src).unwrap_or_else(|e| panic!("{expr}: {e}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    }
}

#[test]
fn weak_and_pin_fail_closed_even_for_scalars() {
    for (expr, code) in [
        ("weak! 42", "E_WEAK_UNSUPPORTED"),
        ("pin! 42", "E_PIN_UNSUPPORTED"),
        ("weak! (1 + 2)", "E_WEAK_UNSUPPORTED"),
        ("pin! (1 + 2)", "E_PIN_UNSUPPORTED"),
    ] {
        let failure = compile(&source(expr)).expect_err(expr);
        assert_eq!(failure.code, code, "{expr}: {failure}");
    }
}

#[test]
fn mutable_qualifier_is_not_silently_accepted_on_new_operations() {
    for op in ["freeze", "weak", "pin"] {
        let src = source(&format!("{op}! mut owner"));
        let failure = compile(&src).expect_err(&src);
        assert_eq!(failure.code, "E_SYNTAX", "{op}: {failure}");
    }
}

#[test]
fn unknown_type_does_not_gain_fake_immutable_or_weak_capability() {
    // No backend code is produced from an unproven constructed value.
    let src = r#"
define class Payload as
  let int value = 7;
end
pub fnc main(): int {
  val item = new Payload();
  val frozen = freeze! item;
  return 0;
}
"#;
    let failure = compile(src).expect_err("class freeze must require a proof");
    // Depending on nominal lowering admission this could fail earlier,
    // but it MUST NOT be accepted as a scalar identity operation.
    assert!(matches!(failure.code, "E_FREEZE_PROOF" | "E_UNSUPPORTED" | "E_UNSUPPORTED_TYPE"),
        "unexpected error: {failure}");
}
