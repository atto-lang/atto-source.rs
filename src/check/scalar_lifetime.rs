//! Conservative non-lexical end-of-loan scan for executable scalar aliases.
//!
//! This is intentionally NOT Polonius Alpha. The source AST supplies the
//! candidate last-use fact directly: no caller-authored lifetime labels or
//! external fact files can shorten a borrow. A loan may be retired only in
//! its declaring lexical scope, before a subsequent straight-line statement,
//! when *every* remaining statement is in the scan's closed expression set
//! and none refers to that alias. Unknown constructs fail closed (retain loan).
//! No change is made to returned references, heap references, loops, actor
//! isolation, async lifetimes, unwind cleanup, or compiler lowering.

use crate::ast::{Expr, ExprKind, Stmt};

const MAX_STRAIGHT_STATEMENTS: usize = 256;
const MAX_VISITED_EXPR_NODES: usize = 4096;

/// True is a conservative proof that the alias is unused in ALL remaining
/// straight-line statements. False means either used or insufficient proof.
pub(super) fn proven_dead_after_boundary(tail: &[Stmt], alias: &str) -> bool {
    if alias.is_empty() || tail.len() > MAX_STRAIGHT_STATEMENTS {
        return false;
    }
    let mut budget = MAX_VISITED_EXPR_NODES;
    for stmt in tail {
        match stmt_uses_alias(stmt, alias, &mut budget) {
            Some(false) => {}
            Some(true) | None => return false,
        }
    }
    true
}

/// None = unsupported or resource budget exhausted, i.e. MAY use alias.
fn expr_uses_alias(expr: &Expr, alias: &str, budget: &mut usize) -> Option<bool> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    match &expr.kind {
        ExprKind::Var(name) => Some(name == alias),
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_)
        | ExprKind::SelfRef | ExprKind::Spawn(_) => Some(false),
        ExprKind::Unary(_, inner) | ExprKind::Trap(inner) | ExprKind::Propagate(inner)
        | ExprKind::Member(inner, _) => expr_uses_alias(inner, alias, budget),
        ExprKind::Binary(_, a, b) => combine([
            expr_uses_alias(a, alias, budget),
            expr_uses_alias(b, alias, budget),
        ]),
        ExprKind::Ternary(a, b, c) => combine([
            expr_uses_alias(a, alias, budget),
            expr_uses_alias(b, alias, budget),
            expr_uses_alias(c, alias, budget),
        ]),
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) =>
            expr_list_uses_alias(args, alias, budget),
        ExprKind::MethodCall(recv, _, args) => {
            let recv = expr_uses_alias(recv, alias, budget);
            // Always scan all arguments, including those with side effects.
            let args = expr_list_uses_alias(args, alias, budget);
            combine([recv, args])
        }
        // Callables/captures cannot be certified without typed environment.
        ExprKind::Lambda { .. } => None,
    }
}
fn combine<const N: usize>(items: [Option<bool>; N]) -> Option<bool> {
    if items.iter().any(|x| *x == Some(true)) {
        Some(true)
    } else if items.iter().any(Option::is_none) {
        None
    } else {
        Some(false)
    }
}
fn expr_list_uses_alias(items: &[Expr], alias: &str, budget: &mut usize) -> Option<bool> {
    let mut unknown = false;
    for item in items {
        match expr_uses_alias(item, alias, budget) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown { None } else { Some(false) }
}
fn stmt_uses_alias(stmt: &Stmt, alias: &str, budget: &mut usize) -> Option<bool> {
    match stmt {
        Stmt::Bind { init, .. } => expr_uses_alias(init, alias, budget),
        Stmt::Assign { name, value, .. } => {
            let rhs = expr_uses_alias(value, alias, budget);
            combine([Some(name == alias), rhs])
        }
        Stmt::FieldAssign { target, value, .. } => combine([
            expr_uses_alias(target, alias, budget),
            expr_uses_alias(value, alias, budget),
        ]),
        Stmt::Expr(value) | Stmt::Return(Some(value), _) =>
            expr_uses_alias(value, alias, budget),
        Stmt::Return(None, _) => Some(false),
        // If, match, blocks, loop, throw, yield or abrupt exits require
        // actual CFG/cleanup provenance. The current V1 does not release
        // loans based on an approximation of those paths.
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinOp, BindKind, ExprKind};
    use crate::diag::Pos;

    fn expr(kind: ExprKind) -> Expr {
        Expr { kind, pos: Pos::default() }
    }
    fn var(name: &str) -> Expr {
        expr(ExprKind::Var(name.to_string()))
    }
    fn lit(value: u64) -> Expr {
        expr(ExprKind::Int(Some(value)))
    }
    fn assign(name: &str, value: Expr) -> Stmt {
        Stmt::Assign { name: name.to_string(), value, pos: Pos::default() }
    }

    #[test]
    fn no_alias_use_in_remaining_straight_line_proves_dead() {
        assert!(proven_dead_after_boundary(&[
            assign("owner", lit(42)),
            Stmt::Return(Some(var("owner")), Pos::default()),
        ], "reader"));
    }

    #[test]
    fn copy_and_return_alias_keeps_loan_live() {
        assert!(!proven_dead_after_boundary(&[
            Stmt::Return(Some(expr(ExprKind::Unary(
                crate::ast::UnOp::Copy,
                Box::new(var("reader")),
            ))), Pos::default()),
        ], "reader"));
    }

    #[test]
    fn assignment_through_exclusive_alias_keeps_loan_live() {
        assert!(!proven_dead_after_boundary(&[assign("writer", lit(41))], "writer"));
    }

    #[test]
    fn nested_condition_without_verified_cfg_retains_loan() {
        assert!(!proven_dead_after_boundary(&[
            Stmt::If {
                arms: vec![(expr(ExprKind::Bool(true)), vec![Stmt::Expr(var("reader"))])],
                otherwise: None,
                pos: Pos::default(),
            },
        ], "reader"));
    }

    #[test]
    fn calls_nested_binary_and_ternary_operands_are_fully_scanned() {
        let call = expr(ExprKind::Call(
            "inspect".to_string(),
            vec![expr(ExprKind::Binary(
                BinOp::Add,
                Box::new(lit(2)),
                Box::new(expr(ExprKind::Ternary(
                    Box::new(expr(ExprKind::Bool(true))),
                    Box::new(lit(5)),
                    Box::new(var("reader")),
                ))),
            ))],
        ));
        assert!(!proven_dead_after_boundary(&[Stmt::Expr(call)], "reader"));
    }

    #[test]
    fn initializer_alias_read_cannot_be_omitted_from_scan() {
        assert!(!proven_dead_after_boundary(&[
            Stmt::Bind {
                kind: BindKind::Val,
                name: "copy".to_string(),
                ty: None,
                init: var("reader"),
                pos: Pos::default(),
            },
        ], "reader"));
    }

    #[test]
    fn unknown_callable_and_control_flow_fail_closed() {
        assert!(!proven_dead_after_boundary(&[
            Stmt::Expr(expr(ExprKind::Lambda {
                params: vec![],
                return_type: None,
                body: crate::ast::LambdaBody::Expression(Box::new(lit(42))),
            })),
        ], "reader"));
        assert!(!proven_dead_after_boundary(&[
            Stmt::Block(vec![Stmt::Expr(lit(2))]),
        ], "reader"));
    }

    #[test]
    fn oversized_tail_fails_closed_instead_of_scanning_unbounded_source() {
        let stmts: Vec<Stmt> = (0..257).map(|_| Stmt::Expr(lit(42))).collect();
        assert!(!proven_dead_after_boundary(&stmts, "reader"));
        assert!(!proven_dead_after_boundary(&[], ""));
    }
}
