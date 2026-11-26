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
/// Hard upper bound across ALL lifetime scans in one checked function;
/// prevents many alias/suffix combinations from causing quadratic work.
pub(super) const MAX_NLL_WORK_PER_FUNCTION: usize = 32_768;

/// True is a conservative proof that the alias is unused in ALL remaining
/// straight-line statements. False means either used or insufficient proof.
pub(super) fn proven_dead_after_boundary(
    tail: &[Stmt],
    alias: &str,
    function_budget: &mut usize,
) -> bool {
    if alias.is_empty() || tail.len() > MAX_STRAIGHT_STATEMENTS || *function_budget == 0 {
        return false;
    }
    // Keep each individual scan finite as well as the entire function.
    let mut scan_budget = (*function_budget).min(MAX_VISITED_EXPR_NODES);
    let original = scan_budget;
    let mut result = true;
    for stmt in tail {
        if scan_budget == 0 {
            result = false;
            break;
        }
        scan_budget -= 1;
        match stmt_uses_alias(stmt, alias, &mut scan_budget) {
            Some(false) => {}
            Some(true) | None => {
                result = false;
                break;
            }
        }
    }
    *function_budget -= original - scan_budget;
    result
}

/// None = unsupported or resource budget exhausted, i.e. MAY use alias.
fn expr_uses_alias(expr: &Expr, alias: &str, budget: &mut usize) -> Option<bool> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    match &expr.kind {
        ExprKind::Var(name) => Some(name == alias),
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::SelfRef => Some(false),
        ExprKind::Unary(
            crate::ast::UnOp::Neg
            | crate::ast::UnOp::Plus
            | crate::ast::UnOp::Not
            | crate::ast::UnOp::Copy,
            inner,
        ) => expr_uses_alias(inner, alias, budget),
        // A checked scalar borrow or take of a named local has no
        // unproved guest-side effect: bind_scalar_borrow / scalar_take
        // validate mutability, moves and conflicts. The old alias itself
        // remains a live use; computed targets cannot authorize release.
        ExprKind::Unary(
            crate::ast::UnOp::Borrow | crate::ast::UnOp::BorrowMut | crate::ast::UnOp::Take,
            inner,
        ) => {
            if matches!(&inner.kind, ExprKind::Var(_)) {
                expr_uses_alias(inner, alias, budget)
            } else {
                None
            }
        }
        ExprKind::Binary(_, a, b) => combine([
            expr_uses_alias(a, alias, budget),
            expr_uses_alias(b, alias, budget),
        ]),
        ExprKind::Ternary(a, b, c) => combine([
            expr_uses_alias(a, alias, budget),
            expr_uses_alias(b, alias, budget),
            expr_uses_alias(c, alias, budget),
        ]),
        // A future call may throw or run an unmodeled cleanup path.
        // Actor spawns, ownership transfer, propagation, field/heap accesses,
        // callbacks and captures require a real typed CFG/effect proof.
        // Retaining the lexical loan is safer than claiming source purity.
        ExprKind::Call(..)
        | ExprKind::MethodCall(..)
        | ExprKind::New(..)
        | ExprKind::Array(..)
        | ExprKind::Spawn(..)
        | ExprKind::Trap(..)
        | ExprKind::Propagate(..)
        | ExprKind::Member(..)
        | ExprKind::Lambda { .. } => None,
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
        Stmt::Expr(value) | Stmt::Return(Some(value), _) => expr_uses_alias(value, alias, budget),
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

    fn proves(tail: &[Stmt], alias: &str) -> bool {
        let mut budget = MAX_VISITED_EXPR_NODES;
        proven_dead_after_boundary(tail, alias, &mut budget)
    }

    fn expr(kind: ExprKind) -> Expr {
        Expr {
            kind,
            pos: Pos::default(),
        }
    }
    fn var(name: &str) -> Expr {
        expr(ExprKind::Var(name.to_string()))
    }
    fn lit(value: u64) -> Expr {
        expr(ExprKind::Int(Some(value)))
    }
    fn assign(name: &str, value: Expr) -> Stmt {
        Stmt::Assign {
            name: name.to_string(),
            value,
            pos: Pos::default(),
        }
    }

    #[test]
    fn no_alias_use_in_remaining_straight_line_proves_dead() {
        assert!(proves(
            &[
                assign("owner", lit(42)),
                Stmt::Return(Some(var("owner")), Pos::default()),
            ],
            "reader"
        ));
    }

    #[test]
    fn copy_and_return_alias_keeps_loan_live() {
        assert!(!proves(
            &[Stmt::Return(
                Some(expr(ExprKind::Unary(
                    crate::ast::UnOp::Copy,
                    Box::new(var("reader")),
                ))),
                Pos::default()
            ),],
            "reader"
        ));
    }

    #[test]
    fn assignment_through_exclusive_alias_keeps_loan_live() {
        assert!(!proves(&[assign("writer", lit(41))], "writer"));
    }

    #[test]
    fn nested_condition_without_verified_cfg_retains_loan() {
        assert!(!proves(
            &[Stmt::If {
                arms: vec![(expr(ExprKind::Bool(true)), vec![Stmt::Expr(var("reader"))])],
                otherwise: None,
                pos: Pos::default(),
            },],
            "reader"
        ));
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
        assert!(!proves(&[Stmt::Expr(call)], "reader"));
    }

    #[test]
    fn initializer_alias_read_cannot_be_omitted_from_scan() {
        assert!(!proves(
            &[Stmt::Bind {
                kind: BindKind::Val,
                name: "copy".to_string(),
                ty: None,
                init: var("reader"),
                pos: Pos::default(),
            },],
            "reader"
        ));
    }

    #[test]
    fn unknown_callable_and_control_flow_fail_closed() {
        assert!(!proves(
            &[Stmt::Expr(expr(ExprKind::Lambda {
                params: vec![],
                return_type: None,
                body: crate::ast::LambdaBody::Expression(Box::new(lit(42))),
            })),],
            "reader"
        ));
        assert!(!proves(&[Stmt::Block(vec![Stmt::Expr(lit(2))]),], "reader"));
    }

    #[test]
    fn unmodeled_effects_cannot_prove_loan_dead_even_without_alias_mentions() {
        let effects = vec![
            ExprKind::Call("might_throw".to_string(), vec![lit(1)]),
            ExprKind::New("Object".to_string(), vec![]),
            ExprKind::Spawn("Worker".to_string()),
            ExprKind::MethodCall(Box::new(var("other")), "send".to_string(), vec![lit(2)]),
            ExprKind::Trap(Box::new(var("other"))),
            ExprKind::Propagate(Box::new(var("other"))),
            ExprKind::Member(Box::new(var("other")), "state".to_string()),
            ExprKind::Array(vec![lit(3)]),
        ];
        for effect in effects {
            assert!(!proves(&[Stmt::Expr(expr(effect))], "reader"));
        }
    }

    #[test]
    fn checked_named_ownership_forms_do_not_poison_completed_scalar_loan() {
        use crate::ast::UnOp;
        for operation in [UnOp::Borrow, UnOp::BorrowMut, UnOp::Take] {
            // New borrow or move of a named owner is allowed after last use.
            assert!(proves(
                &[Stmt::Bind {
                    kind: BindKind::Val,
                    name: "writer".to_string(),
                    ty: None,
                    init: expr(ExprKind::Unary(operation, Box::new(var("owner")))),
                    pos: Pos::default(),
                }],
                "reader",
            ));
            // New borrow or take of the alias itself is still a live use.
            assert!(!proves(
                &[Stmt::Expr(expr(ExprKind::Unary(
                    operation,
                    Box::new(var("reader")),
                )))],
                "reader",
            ));
            // No permission to borrow or take a computed/effectful expression.
            assert!(!proves(
                &[Stmt::Expr(expr(ExprKind::Unary(
                    operation,
                    Box::new(expr(ExprKind::Call("get_owner".to_string(), vec![],))),
                )))],
                "reader",
            ));
        }
    }

    #[test]
    fn pure_nested_operands_without_alias_still_can_end_a_loan() {
        let e = expr(ExprKind::Ternary(
            Box::new(expr(ExprKind::Bool(true))),
            Box::new(expr(ExprKind::Binary(
                BinOp::Add,
                Box::new(lit(40)),
                Box::new(lit(2)),
            ))),
            Box::new(lit(0)),
        ));
        assert!(proves(&[Stmt::Expr(e)], "reader"));
    }

    #[test]
    fn shared_function_budget_is_consumed_and_exhaustion_is_fail_closed() {
        let single = [Stmt::Expr(lit(42))];
        let mut remaining = 5;
        assert!(proven_dead_after_boundary(
            &single,
            "reader",
            &mut remaining
        ));
        assert_eq!(remaining, 3);
        assert!(!proven_dead_after_boundary(
            &[Stmt::Expr(lit(1)), Stmt::Expr(lit(2))],
            "reader",
            &mut remaining,
        ));
        assert_eq!(remaining, 0);
        assert!(!proven_dead_after_boundary(
            &single,
            "reader",
            &mut remaining
        ));
    }

    #[test]
    fn single_scan_budget_and_statement_count_are_both_bounded() {
        let stmts: Vec<Stmt> = (0..MAX_STRAIGHT_STATEMENTS + 1)
            .map(|_| Stmt::Expr(lit(42)))
            .collect();
        let mut budget = MAX_NLL_WORK_PER_FUNCTION;
        assert!(!proven_dead_after_boundary(&stmts, "reader", &mut budget));
        assert_eq!(budget, MAX_NLL_WORK_PER_FUNCTION);
    }

    #[test]
    fn oversized_tail_fails_closed_instead_of_scanning_unbounded_source() {
        let stmts: Vec<Stmt> = (0..257).map(|_| Stmt::Expr(lit(42))).collect();
        assert!(!proves(&stmts, "reader"));
        assert!(!proves(&[], ""));
    }
}
