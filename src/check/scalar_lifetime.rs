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
