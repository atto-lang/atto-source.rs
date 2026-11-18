//! Conservative intra-scope last-use analysis for existing class loans.
//!
//! This analysis deliberately counts *all* syntactic uses, including inside
//! branches, loops, lambda bodies and deferred bodies, even when a path might
//! be unreachable or a name is shadowed. Over-approximation keeps a loan alive
//! longer; under-approximation would make ownership checking unsound.
//!
//! Not a general NLL solver: there is no returned-borrow provenance, indirect
//! closure capture, suspension or cross-function lifetime contract here.
use crate::ast::{Expr, ExprKind, LambdaBody, Stmt};

pub(super) fn stmt_mentions(stmt: &Stmt, target: &str) -> bool {
    match stmt {
        Stmt::Bind { init, .. } => expr_mentions(init, target),
        Stmt::Assign { name, value, .. } =>
            name == target || expr_mentions(value, target),
        Stmt::StateWrite { body, .. } | Stmt::Block(body) |
        Stmt::Loop { body, .. } =>
            body.iter().any(|s| stmt_mentions(s, target)),
        Stmt::FieldAssign { target: owner, value, .. } =>
            expr_mentions(owner, target) || expr_mentions(value, target),
        Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Raise(e, _) |
        Stmt::Yield(e, _) | Stmt::YieldFrom(e, _) =>
            expr_mentions(e, target),
        Stmt::Try { body, catch, finally, .. } =>
            body.iter().any(|s| stmt_mentions(s, target)) ||
            catch.as_ref().is_some_and(|(_, b)| b.iter().any(|s| stmt_mentions(s, target))) ||
            finally.as_ref().is_some_and(|b| b.iter().any(|s| stmt_mentions(s, target))),
        Stmt::Defer { body, .. } =>
            body.iter().any(|s| stmt_mentions(s, target)),
        Stmt::Recover { args, body, .. } =>
            args.iter().any(|e| expr_mentions(e, target)) ||
            body.iter().any(|s| stmt_mentions(s, target)),
        Stmt::Return(e, _) => e.as_ref().is_some_and(|e| expr_mentions(e, target)),
        Stmt::If { arms, otherwise, .. } =>
            arms.iter().any(|(e, b)| expr_mentions(e, target) ||
                b.iter().any(|s| stmt_mentions(s, target))) ||
            otherwise.as_ref().is_some_and(|b| b.iter().any(|s| stmt_mentions(s, target))),
        Stmt::While { cond, body, .. } =>
            expr_mentions(cond, target) || body.iter().any(|s| stmt_mentions(s, target)),
        Stmt::For { init, cond, update, body, .. } =>
            init.as_ref().is_some_and(|s| stmt_mentions(s, target)) ||
            cond.as_ref().is_some_and(|e| expr_mentions(e, target)) ||
            update.as_ref().is_some_and(|s| stmt_mentions(s, target)) ||
            body.iter().any(|s| stmt_mentions(s, target)),
        Stmt::ForIn { source, body, .. } =>
            expr_mentions(source, target) || body.iter().any(|s| stmt_mentions(s, target)),
        Stmt::Match { subject, arms, .. } =>
            expr_mentions(subject, target) ||
            arms.iter().any(|a| a.guard.as_ref().is_some_and(|e| expr_mentions(e, target)) ||
                a.body.iter().any(|s| stmt_mentions(s, target))),
        Stmt::Break(_) | Stmt::Continue(_) => false,
    }
}

fn expr_mentions(expr: &Expr, target: &str) -> bool {
    match &expr.kind {
        ExprKind::Var(name) => name == target,
        ExprKind::Unary(_, e) | ExprKind::Trap(e) |
        ExprKind::Ownership(_, _, e) | ExprKind::Propagate(e) |
        ExprKind::Member(e, _) => expr_mentions(e, target),
        ExprKind::Binary(_, lhs, rhs) =>
            expr_mentions(lhs, target) || expr_mentions(rhs, target),
        ExprKind::Ternary(a, b, c) =>
            expr_mentions(a, target) || expr_mentions(b, target) ||
            expr_mentions(c, target),
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) =>
            args.iter().any(|e| expr_mentions(e, target)),
        ExprKind::MethodCall(receiver, _, args) =>
            expr_mentions(receiver, target) ||
            args.iter().any(|e| expr_mentions(e, target)),
        ExprKind::Lambda { body, .. } => match body {
            LambdaBody::Expression(e) => expr_mentions(e, target),
            LambdaBody::Statement(s) => stmt_mentions(s, target),
            LambdaBody::Block(body) => body.iter().any(|s| stmt_mentions(s, target)),
        },
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) |
        ExprKind::Str(_) | ExprKind::SelfRef | ExprKind::StateRead(_) |
        ExprKind::Spawn(_) => false,
    }
}
