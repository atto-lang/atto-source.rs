//! Classify *provable* signed-i64 overflow in explicitly constant contexts.
//!
//! This observes the already typechecked expression tree. It does not fold or
//! rewrite emitted guest IR, and must never be run on ordinary val/let/return
//! expressions: those retain their checked runtime trap edges.
//! A None result means "not proven constant", never "safe to wrap".

use std::collections::BTreeMap;

use crate::ast::BinOp;
use crate::check::{Builtin, TE, TExpr};
use crate::diag::{Diagnostic, Pos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scalar {
    Int(i64),
    Bool(bool),
}

fn overflow(pos: Pos) -> Diagnostic {
    Diagnostic::new(pos, "E_CONST_OVERFLOW",
        "signed 64-bit arithmetic overflow in a compile-time constant")
}

fn checked(value: Option<i64>, pos: Pos) -> Result<Option<Scalar>, Diagnostic> {
    value.map(Scalar::Int).map(Some).ok_or_else(|| overflow(pos))
}

/// Evaluate the subset of typechecked, pure scalars that is provable without
/// executing a guest. All binary inputs are inspected, including an overflow
/// under multiplication by zero. A ternary inspects only the selected arm
/// when its condition is statically known, matching guest laziness.
pub(crate) fn evaluate(
    expr: &TE, known: &BTreeMap<u32, Scalar>, pos: Pos,
) -> Result<Option<Scalar>, Diagnostic> {
    use Scalar::{Bool, Int};
    Ok(match &expr.e {
        TExpr::I64(n) => Some(Int(*n)),
        TExpr::Bool(v) => Some(Bool(*v)),
        TExpr::Local(slot) => known.get(slot).copied(),
        TExpr::Neg(inner) => match evaluate(inner, known, pos)? {
            Some(Int(n)) => return checked(n.checked_neg(), pos),
            _ => None,
        },
        TExpr::Not(inner) => match evaluate(inner, known, pos)? {
            Some(Bool(b)) => Some(Bool(!b)),
            _ => None,
        },
        TExpr::Bin(op, lhs, rhs) => {
            // Eager arithmetic operands must both be validated even if one is
            // dynamic or the eventual computed value would be discarded.
            let left = evaluate(lhs, known, pos)?;
            let right = evaluate(rhs, known, pos)?;
            match (left, right) {
                (Some(Int(a)), Some(Int(b))) => match op {
                    BinOp::Add => return checked(a.checked_add(b), pos),
                    BinOp::Sub => return checked(a.checked_sub(b), pos),
                    BinOp::Mul => return checked(a.checked_mul(b), pos),
                    BinOp::BitAnd => Some(Int(a & b)),
                    BinOp::BitOr => Some(Int(a | b)),
                    BinOp::BitXor => Some(Int(a ^ b)),
                    BinOp::Shl => Some(Int(a.wrapping_shl((b as u64 & 63) as u32))),
                    BinOp::LShr => Some(Int(((a as u64) >> (b as u64 & 63)) as i64)),
                    BinOp::AShr => Some(Int(a >> (b as u64 & 63))),
                    BinOp::Eq => Some(Bool(a == b)),
                    BinOp::Ne => Some(Bool(a != b)),
                    BinOp::Lt => Some(Bool(a < b)),
                    BinOp::Le => Some(Bool(a <= b)),
                    BinOp::Gt => Some(Bool(a > b)),
                    BinOp::Ge => Some(Bool(a >= b)),
                    _ => None,
                },
                (Some(Bool(a)), Some(Bool(b))) => match op {
                    BinOp::Eq => Some(Bool(a == b)),
                    BinOp::Ne => Some(Bool(a != b)),
                    BinOp::And => Some(Bool(a && b)),
                    BinOp::Or => Some(Bool(a || b)),
                    _ => None,
                },
                _ => None,
            }
        }
        TExpr::Ternary(condition, yes, no) => match evaluate(condition, known, pos)? {
            Some(Bool(true)) => evaluate(yes, known, pos)?,
            Some(Bool(false)) => evaluate(no, known, pos)?,
            _ => None, // Unproven selector: runtime checked traps stay in guest.
        },
        TExpr::Builtin(kind, lhs, rhs) => {
            let left = evaluate(lhs, known, pos)?;
            match (kind, left) {
                (Builtin::And, Some(Bool(false))) => Some(Bool(false)),
                (Builtin::Or, Some(Bool(true))) => Some(Bool(true)),
                (Builtin::And, Some(Bool(true))) => match evaluate(rhs, known, pos)? {
                    Some(Bool(b)) => Some(Bool(b)),
                    _ => None,
                },
                (Builtin::Or, Some(Bool(false))) => match evaluate(rhs, known, pos)? {
                    Some(Bool(b)) => Some(Bool(b)),
                    _ => None,
                },
                (Builtin::Xor, Some(Bool(a))) => match evaluate(rhs, known, pos)? {
                    Some(Bool(b)) => Some(Bool(a ^ b)),
                    _ => None,
                },
                // Unknown short-circuit input cannot prove whether RHS runs.
                _ => None,
            }
        }
        TExpr::IntToFloat(inner) | TExpr::FNeg(inner) => {
            // Validate nested known signed arithmetic without pretending to
            // implement a complete float constexpr evaluator.
            let _ = evaluate(inner, known, pos)?;
            None
        }
        TExpr::Str(_, lhs, rhs) => {
            let _ = evaluate(lhs, known, pos)?;
            if let Some(rhs) = rhs {
                let _ = evaluate(rhs, known, pos)?;
            }
            None
        }
        // Calls, heap values, I/O and other effects are deliberately not
        // evaluated here. This pass is an overflow classifier, not a complete
        // constexpr-purity checker or a new guest execution engine.
        _ => None,
    })
}
