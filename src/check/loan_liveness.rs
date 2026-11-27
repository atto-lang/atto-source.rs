//! Conservative intra-scope last-use analysis for class and scalar field loans.
//!
//! This analysis deliberately counts *all* syntactic uses, including inside
//! branches, loops, lambda bodies and deferred bodies, even when a path might
//! be unreachable or a name is shadowed. Over-approximation keeps a loan alive
//! longer; under-approximation would make ownership checking unsound.
//!
//! Not a general NLL solver: there is no returned-borrow provenance, indirect
//! closure capture, suspension or cross-function lifetime contract here.
use crate::ast::{Expr, ExprKind, LambdaBody, Stmt};

/// Reuse a previously discovered possible future use while it remains ahead
/// of the current statement. A witness may be conservative (budget exhausted);
/// it never licenses an early loan end. Each scan advances past its old witness.
/// The map belongs to one immutable statement body, keyed by compiler slots.
pub(super) fn has_future_use(
    body: &[Stmt],
    current: usize,
    slot: u32,
    name: &str,
    fuel: &mut usize,
    witnesses: &mut std::collections::BTreeMap<u32, usize>,
) -> bool {
    if witnesses.get(&slot).is_some_and(|&next| next > current) {
        return true;
    }
    witnesses.remove(&slot);
    for (offset, stmt) in body[current + 1..].iter().enumerate() {
        if stmt_mentions(stmt, name, fuel) {
            witnesses.insert(slot, current + 1 + offset);
            return true;
        }
    }
    false
}

pub(super) fn stmt_mentions(stmt: &Stmt, target: &str, fuel: &mut usize) -> bool {
    if *fuel == 0 {
        return true;
    } // unknown means possibly used: retain the loan
    *fuel -= 1;
    match stmt {
        Stmt::Bind { init, .. } => expr_mentions(init, target, fuel),
        Stmt::Assign { name, value, .. } => name == target || expr_mentions(value, target, fuel),
        Stmt::StateWrite { body, .. } | Stmt::Block(body) | Stmt::Loop { body, .. } => {
            body.iter().any(|s| stmt_mentions(s, target, fuel))
        }
        Stmt::FieldAssign {
            target: owner,
            value,
            ..
        } => expr_mentions(owner, target, fuel) || expr_mentions(value, target, fuel),
        Stmt::Expr(e)
        | Stmt::Throw(e, _)
        | Stmt::Raise(e, _)
        | Stmt::Yield(e, _)
        | Stmt::YieldFrom(e, _) => expr_mentions(e, target, fuel),
        Stmt::Try {
            body,
            catch,
            finally,
            ..
        } => {
            body.iter().any(|s| stmt_mentions(s, target, fuel))
                || catch
                    .as_ref()
                    .is_some_and(|(_, b)| b.iter().any(|s| stmt_mentions(s, target, fuel)))
                || finally
                    .as_ref()
                    .is_some_and(|b| b.iter().any(|s| stmt_mentions(s, target, fuel)))
        }
        Stmt::Defer { args, body, .. } => {
            args.iter().any(|e| expr_mentions(e, target, fuel))
                || body.iter().any(|s| stmt_mentions(s, target, fuel))
        },
        Stmt::Recover { args, body, .. } => {
            args.iter().any(|e| expr_mentions(e, target, fuel))
                || body.iter().any(|s| stmt_mentions(s, target, fuel))
        }
        Stmt::Return(e, _) => e.as_ref().is_some_and(|e| expr_mentions(e, target, fuel)),
        Stmt::If {
            arms, otherwise, ..
        } => {
            arms.iter().any(|(e, b)| {
                expr_mentions(e, target, fuel) || b.iter().any(|s| stmt_mentions(s, target, fuel))
            }) || otherwise
                .as_ref()
                .is_some_and(|b| b.iter().any(|s| stmt_mentions(s, target, fuel)))
        }
        Stmt::While { cond, body, .. } => {
            expr_mentions(cond, target, fuel) || body.iter().any(|s| stmt_mentions(s, target, fuel))
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref()
                .is_some_and(|s| stmt_mentions(s, target, fuel))
                || cond
                    .as_ref()
                    .is_some_and(|e| expr_mentions(e, target, fuel))
                || update
                    .as_ref()
                    .is_some_and(|s| stmt_mentions(s, target, fuel))
                || body.iter().any(|s| stmt_mentions(s, target, fuel))
        }
        Stmt::ForIn { source, body, .. } => {
            expr_mentions(source, target, fuel)
                || body.iter().any(|s| stmt_mentions(s, target, fuel))
        }
        Stmt::Match { subject, arms, .. } => {
            expr_mentions(subject, target, fuel)
                || arms.iter().any(|a| {
                    a.guard
                        .as_ref()
                        .is_some_and(|e| expr_mentions(e, target, fuel))
                        || a.body.iter().any(|s| stmt_mentions(s, target, fuel))
                })
        }
        // Telemetry scopes are newer than the original lifetime profile.
        // Retain every live loan until their effect and cleanup census is proven.
        Stmt::UsingTelemetry { .. } => true,
        Stmt::Break(_) | Stmt::Continue(_) => false,
    }
}

fn expr_mentions(expr: &Expr, target: &str, fuel: &mut usize) -> bool {
    if *fuel == 0 {
        return true;
    }
    *fuel -= 1;
    match &expr.kind {
        ExprKind::Var(name) => name == target,
        ExprKind::Await(e) => expr_mentions(e, target, fuel),
        ExprKind::SpawnTask { args, .. } => args.iter().any(|a| expr_mentions(a, target, fuel)),
        // A task body is a separate callable; captures are by-value copies.
        ExprKind::Task { .. } | ExprKind::Duration(_) => false,
        ExprKind::Unary(_, e)
        | ExprKind::Trap(e)
        | ExprKind::Ownership(_, _, e)
        | ExprKind::Propagate(e)
        | ExprKind::Ascribe(e, _, _)
        | ExprKind::Member(e, _) => expr_mentions(e, target, fuel),
        ExprKind::Binary(_, lhs, rhs) => {
            expr_mentions(lhs, target, fuel) || expr_mentions(rhs, target, fuel)
        }
        ExprKind::Ternary(a, b, c) => {
            expr_mentions(a, target, fuel)
                || expr_mentions(b, target, fuel)
                || expr_mentions(c, target, fuel)
        }
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) => {
            args.iter().any(|e| expr_mentions(e, target, fuel))
        }
        ExprKind::MethodCall(receiver, _, args) => {
            expr_mentions(receiver, target, fuel)
                || args.iter().any(|e| expr_mentions(e, target, fuel))
        }
        ExprKind::Lambda { body, .. } => match body {
            LambdaBody::Expression(e) => expr_mentions(e, target, fuel),
            LambdaBody::Statement(s) => stmt_mentions(s, target, fuel),
            LambdaBody::Block(body) => body.iter().any(|s| stmt_mentions(s, target, fuel)),
        },
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::SelfRef
        | ExprKind::StateRead(_)
        | ExprKind::Spawn(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Pos;

    #[test]
    fn type_ascription_and_explicit_cast_preserve_nested_loan_use() {
        // Ascribe was added on modern main after this loan visitor forked.
        // A loan read under either spelling is still live at this statement.
        let p = Pos { line: 1, col: 1 };
        for explicit in [false, true] {
            let use_loan = Expr {
                kind: ExprKind::Ascribe(
                    Box::new(Expr { kind: ExprKind::Var("reader".into()), pos: p }),
                    crate::ast::Type::Int,
                    explicit,
                ),
                pos: p,
            };
            let mut fuel = 32;
            assert!(stmt_mentions(&Stmt::Expr(use_loan), "reader", &mut fuel));
        }
    }

    #[test]
    fn depleted_budget_preserves_a_loan_even_with_no_apparent_use() {
        let literal = Expr {
            kind: ExprKind::Int(Some(42)),
            pos: Pos { line: 1, col: 1 },
        };
        let mut fuel = 0;
        assert!(stmt_mentions(&Stmt::Expr(literal), "reader", &mut fuel));
    }

    #[test]
    fn ample_budget_can_prove_a_literal_has_no_loan_use() {
        let literal = Expr {
            kind: ExprKind::Int(Some(42)),
            pos: Pos { line: 1, col: 1 },
        };
        let mut fuel = 8;
        assert!(!stmt_mentions(&Stmt::Expr(literal), "reader", &mut fuel));
    }

    #[test]
    fn unknown_telemetry_effects_conservatively_retain_loans() {
        let stmt = Stmt::UsingTelemetry {
            operation: "test".into(),
            alias: "span".into(),
            body: Vec::new(),
            pos: Pos { line: 1, col: 1 },
        };
        let mut fuel = 32;
        assert!(stmt_mentions(&stmt, "reader", &mut fuel));
    }
    #[test]
    fn witness_reuse_does_not_rescan_long_gaps_or_expire_before_use() {
        let pos = Pos::default();
        let literal = Stmt::Expr(Expr { kind: ExprKind::Int(Some(42)), pos });
        let read = Stmt::Expr(Expr { kind: ExprKind::Var("view".into()), pos });
        let mut body = vec![literal; 500];
        body.push(read);
        let mut fuel = 2000;
        let mut witnesses = std::collections::BTreeMap::new();
        assert!(has_future_use(&body, 0, 7, "view", &mut fuel, &mut witnesses));
        let after_first = fuel;
        for i in 1..500 {
            assert!(has_future_use(&body, i, 7, "view", &mut fuel, &mut witnesses));
        }
        assert_eq!(fuel, after_first);
        assert!(!has_future_use(&body, 500, 7, "view", &mut fuel, &mut witnesses));
        assert!(witnesses.is_empty());
    }

    #[test]
    fn uncertain_witness_never_grants_early_retirement() {
        let pos = Pos::default();
        let body = vec![Stmt::Expr(Expr { kind: ExprKind::Int(Some(1)), pos }); 4];
        let mut fuel = 0;
        let mut witnesses = std::collections::BTreeMap::new();
        for i in 0..3 {
            assert!(has_future_use(&body, i, 1, "view", &mut fuel, &mut witnesses));
        }
        assert!(!has_future_use(&body, 3, 1, "view", &mut fuel, &mut witnesses));
    }

}
