//! Narrow executable scalar-receiver CFG. Every admitted method has a single
//! normal return path plus an explicit checked-overflow panic exit.
//!
//! This verifies only *lease-free* scalar instructions. It is not a verifier
//! for stateful actor cleanup, scheduler cancellation, or emitted machine code.
use crate::diag::{Diagnostic, Pos};
use crate::target_scalar::ScalarExpr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarOp {
    Param(u32),
    Const(i64),
    CheckedNeg,
    CheckedAdd,
    CheckedSub,
    CheckedMul,
    Return,
    OverflowPanic,
}

impl ScalarOp {
    fn is_checked(self) -> bool {
        matches!(self, Self::CheckedNeg | Self::CheckedAdd | Self::CheckedSub | Self::CheckedMul)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScalarNode {
    pub(crate) op: ScalarOp,
    // The normal successor follows evaluation order and must be contiguous.
    normal: Option<usize>,
    // All checked instructions must target the same explicit overflow exit.
    overflow: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedScalarCfg {
    nodes: Vec<ScalarNode>,
}

impl VerifiedScalarCfg {
    pub(crate) fn nodes(&self) -> &[ScalarNode] {
        &self.nodes
    }
}

fn invalid(pos: Pos) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", "invalid or unproved scalar receiver instruction CFG")
}

fn verify(nodes: &[ScalarNode], arity: usize, pos: Pos) -> Result<(), Diagnostic> {
    if nodes.len() < 2 || nodes.len() > 130 || arity > 8 {
        return Err(invalid(pos));
    }
    let panic_at = if nodes.last().is_some_and(|n| n.op == ScalarOp::OverflowPanic) {
        Some(nodes.len() - 1)
    } else {
        None
    };
    let normal_len = panic_at.unwrap_or(nodes.len());
    if normal_len < 2
        || nodes[normal_len - 1].op != ScalarOp::Return
        || nodes[..normal_len - 1].iter().any(|n| {
            matches!(n.op, ScalarOp::Return | ScalarOp::OverflowPanic)
        })
    {
        return Err(invalid(pos));
    }
    if let Some(idx) = panic_at {
        if nodes[idx].normal.is_some() || nodes[idx].overflow.is_some() {
            return Err(invalid(pos));
        }
    }

    let mut stack: usize = 0;
    let mut checked = false;
    for (i, node) in nodes[..normal_len].iter().enumerate() {
        let expected_next = if i + 1 == normal_len { None } else { Some(i + 1) };
        if node.normal != expected_next {
            return Err(invalid(pos));
        }
        if node.op.is_checked() {
            checked = true;
            if node.overflow != panic_at || panic_at.is_none() {
                return Err(invalid(pos));
            }
        } else if node.overflow.is_some() {
            return Err(invalid(pos));
        }
        match node.op {
            ScalarOp::Param(index) if (index as usize) < arity => stack += 1,
            ScalarOp::Const(_) => stack += 1,
            ScalarOp::CheckedNeg if stack >= 1 => {}
            ScalarOp::CheckedAdd | ScalarOp::CheckedSub | ScalarOp::CheckedMul
                if stack >= 2 => stack -= 1,
            ScalarOp::Return if stack == 1 => {}
            _ => return Err(invalid(pos)),
        }
        if stack > 128 {
            return Err(invalid(pos));
        }
    }
    if checked != panic_at.is_some() {
        return Err(invalid(pos));
    }
    Ok(())
}

fn postfix(expr: &ScalarExpr, ops: &mut Vec<ScalarOp>) {
    match expr {
        ScalarExpr::Param(index) => ops.push(ScalarOp::Param(*index)),
        ScalarExpr::Const(value) => ops.push(ScalarOp::Const(*value)),
        ScalarExpr::Neg(x) => {
            postfix(x, ops);
            ops.push(ScalarOp::CheckedNeg);
        }
        ScalarExpr::Add(a, b) | ScalarExpr::Sub(a, b) | ScalarExpr::Mul(a, b) => {
            postfix(a, ops);
            postfix(b, ops);
            ops.push(match expr {
                ScalarExpr::Add(..) => ScalarOp::CheckedAdd,
                ScalarExpr::Sub(..) => ScalarOp::CheckedSub,
                _ => ScalarOp::CheckedMul,
            });
        }
    }
}

/// The only constructor for a verified scalar graph. A graph with an invalid
/// stack, missing overflow exit, forged successor, or unreachable instruction
/// is rejected before Wasm instructions can be emitted.
pub(crate) fn lower_scalar_cfg(
    expr: &ScalarExpr,
    arity: usize,
    pos: Pos,
) -> Result<VerifiedScalarCfg, Diagnostic> {
    let mut ops = Vec::new();
    postfix(expr, &mut ops);
    if ops.is_empty() || ops.len() > 128 {
        return Err(invalid(pos));
    }
    let has_checked = ops.iter().any(|op| op.is_checked());
    let panic_at = if has_checked { Some(ops.len() + 1) } else { None };
    ops.push(ScalarOp::Return);
    if has_checked {
        ops.push(ScalarOp::OverflowPanic);
    }
    let nodes: Vec<_> = ops.iter().enumerate().map(|(i, &op)| {
        ScalarNode {
            op,
            normal: if matches!(op, ScalarOp::Return | ScalarOp::OverflowPanic) {
                None
            } else {
                Some(i + 1)
            },
            overflow: if op.is_checked() { panic_at } else { None },
        }
    }).collect();
    verify(&nodes, arity, pos)?;
    Ok(VerifiedScalarCfg { nodes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos() -> Pos { Pos::default() }

    fn addition() -> ScalarExpr {
        ScalarExpr::Add(Box::new(ScalarExpr::Param(0)), Box::new(ScalarExpr::Const(3)))
    }

    #[test]
    fn scalar_cfg_has_explicit_checked_exit_and_single_return() {
        let cfg = lower_scalar_cfg(&addition(), 1, pos()).unwrap();
        assert_eq!(cfg.nodes.len(), 5);
        assert_eq!(cfg.nodes[2].op, ScalarOp::CheckedAdd);
        assert_eq!(cfg.nodes[2].overflow, Some(4));
        assert_eq!(cfg.nodes[3].op, ScalarOp::Return);
        assert_eq!(cfg.nodes[4].op, ScalarOp::OverflowPanic);
        let plain = lower_scalar_cfg(&ScalarExpr::Param(0), 1, pos()).unwrap();
        assert_eq!(plain.nodes.len(), 2);
        assert_eq!(plain.nodes[1].op, ScalarOp::Return);
    }

    #[test]
    fn scalar_cfg_rejects_missing_or_forged_overflow_and_control_flow() {
        let mut cfg = lower_scalar_cfg(&addition(), 1, pos()).unwrap();
        cfg.nodes[2].overflow = None;
        assert!(verify(&cfg.nodes, 1, pos()).is_err());
        cfg.nodes[2].overflow = Some(4);
        cfg.nodes[0].normal = Some(2);
        assert!(verify(&cfg.nodes, 1, pos()).is_err());
        cfg.nodes[0].normal = Some(1);
        cfg.nodes[4].normal = Some(0);
        assert!(verify(&cfg.nodes, 1, pos()).is_err());
        cfg.nodes[4].normal = None;
        cfg.nodes[1].overflow = Some(4);
        assert!(verify(&cfg.nodes, 1, pos()).is_err());
    }

    #[test]
    fn scalar_cfg_rejects_stack_forgery_and_invalid_parameters() {
        assert!(lower_scalar_cfg(&ScalarExpr::Param(1), 1, pos()).is_err());
        let mut cfg = lower_scalar_cfg(&addition(), 1, pos()).unwrap();
        cfg.nodes[1].op = ScalarOp::CheckedAdd;
        assert!(verify(&cfg.nodes, 1, pos()).is_err());
        cfg.nodes[1].op = ScalarOp::Const(3);
        cfg.nodes[3].op = ScalarOp::Param(0);
        assert!(verify(&cfg.nodes, 1, pos()).is_err());
    }
}
