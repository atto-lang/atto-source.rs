//! Checked-browser vocabulary regression: positive scalar paths and negative
//! object/loan/lifetime paths. Do not confuse parsing with runtime admission.
use atto_compiler::{
    ast::{ExprKind, OwnershipOp, Stmt},
    compile, lexer, parser::Parser,
};

fn expr(source: &str) -> atto_compiler::ast::Expr {
    let program = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let Stmt::Bind { init, .. } = &program.funcs[0].body[0] else {
        panic!("expected first binding")
    };
    init.clone()
}

fn binding(value: &str) -> String {
    format!("pub fnc main(): int {{ val result = {value}; return 0; }}")
}

fn wasm_ok(source: &str) {
    let wasm = compile(source).unwrap_or_else(|err| panic!("{source}: {err}"));
    assert_eq!(&wasm[0..8], b"\0asm\x01\0\0\0");
    assert_eq!(compile(source).unwrap(), wasm, "deterministic emission");
}

fn error(source: &str, code: &str) {
    let e = compile(source).expect_err(source);
    assert_eq!(e.code, code, "{source}: {e}");
}

#[test]
fn copy_double_bang_is_a_distinct_ast_operation() {
    for (spelling, op) in [
        ("borrow! 7", OwnershipOp::Borrow),
        ("copy! 7", OwnershipOp::Copy),
        ("copy!! 7", OwnershipOp::DeepCopy),
        ("share! 7", OwnershipOp::Share),
        ("freeze! 7", OwnershipOp::Freeze),
        ("weak! 7", OwnershipOp::Weak),
        ("pin! 7", OwnershipOp::Pin),
        ("take! 7", OwnershipOp::Take),
        ("drop! 7", OwnershipOp::Drop),
    ] {
        let v = expr(&binding(spelling));
        assert!(matches!(v.kind, ExprKind::Ownership(actual, false, _) if actual == op),
            "{spelling} misparsed");
    }
}

#[test]
fn copy_double_bang_must_be_contiguous_not_copy_bang_then_not() {
    let v = expr(&binding("copy! !false"));
    let ExprKind::Ownership(OwnershipOp::Copy, false, inner) = v.kind else {
        panic!("separated operations must preserve their distinct meanings")
    };
    assert!(matches!(inner.kind, ExprKind::Unary(..)));
}

#[test]
fn immutable_primitives_support_shallow_deep_shared_and_frozen_values() {
    for expression in ["42", "true", r#""immutable""#] {
        // Explicit borrows must bind a named owner; a temporary has no
        // auditable lexical lifetime. Do not relax E_BORROW for literals.
        wasm_ok(&format!(
            "pub fnc main(): int {{ val owner = {expression}; val result = borrow! owner; return 0; }}"
        ));
        error(&binding(&format!("borrow! {expression}")), "E_BORROW");
        for operation in ["copy!", "copy!!", "share!", "freeze!"] {
            wasm_ok(&binding(&format!("{operation} {expression}")));
        }
    }
}

#[test]
fn scalar_drop_consumes_ownership_at_this_program_point() {
    wasm_ok("pub fnc main(): int { let x = 42; drop! x; return 7; }");
    error("pub fnc main(): int { let x = 42; drop! x; return x; }", "E_MOVED");
    error("pub fnc main(): int { let x = 42; drop! x; drop! x; return 0; }", "E_MOVED");
}

#[test]
fn drop_does_not_return_a_value_or_accept_computed_temporaries() {
    error(&binding("drop! 9"), "E_DROP_CONTEXT");
    error("pub fnc main(): int { val x = 1; return drop! x; }", "E_DROP_CONTEXT");
    error("pub fnc main(): int { drop! (1 + 2); return 0; }", "E_DROP_CONTEXT");
}

#[test]
fn class_drop_stays_gated_but_authenticated_flat_copy_is_independent() {
    let class_prefix = "define class Data as let int value = 42; end ";
    error(&format!("{class_prefix}pub fnc main(): int {{ val x = new Data(); drop! x; return 0; }}"),
        "E_DROP_UNSUPPORTED");
    // The separate closed-world copy-plan verifier authenticates this flat
    // concrete class: the compiler allocates an independent recipient root.
    wasm_ok(&format!("{class_prefix}pub fnc main(): int {{ val x = new Data(); val y = copy!! x; return y.value; }}"));
    // A user-defined method named copy_deep is not an AOT copy witness.
    let custom = "define class Parcel as pub let int value = 42; copy_deep(): int { return 1; } end pub fnc main(): int { val x = new Parcel(); val y = copy!! x; return 0; }";
    error(custom, "E_COPY_OVERRIDE");
}

#[test]
fn dropping_while_class_owner_is_borrowed_stays_rejected() {
    let src = r#"define class Data as
  let int value = 42;
end
pub fnc main(): int {
  val owner = new Data();
  val loan = borrow! owner;
  drop! owner;
  return 0;
}"#;
    error(src, "E_BORROW");
}

#[test]
fn explicit_drop_in_repeated_loop_cannot_drop_outer_owner() {
    let src = "pub fnc main(): int { let x = 42; loop { drop! x; break; } return 0; }";
    error(src, "E_MOVED");
}

#[test]
fn named_lifetime_binders_are_never_treated_as_runtime_values() {
    for spelling in ["lt! a", "lifetime! a"] {
        error(&binding(spelling), "E_LIFETIME_CONTEXT");
    }
}

#[test]
fn new_operations_cannot_claim_mutable_qualifiers() {
    for op in ["copy!!", "freeze!", "weak!", "pin!", "drop!"] {
        error(&binding(&format!("{op} mut x")), "E_SYNTAX");
    }
}
