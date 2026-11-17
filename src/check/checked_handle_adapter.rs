//! Fail-closed bridge from the type-admitted, restricted checked-handle source
//! profile to the independent bounded ownership CFG solver.
//!
//! This bridge is called *after* ordinary name/type/loan checking succeeded.
//! It consumes only accepted direct val ref!/deref!/unref! syntax and lexical
//! block/do-if structure. It must not be advertised as a general typed-MIR
//! lowering, full NLL solver, runtime deallocator, or actor pin proof.
//! The standalone solver is pinned to atto-borrow-checker.rs PR #33.

use super::checked_handle_cfg::{self as proof, Action, Block, FlowError, MAX_CFG_NODES};
use crate::ast::{ExprKind, OwnershipOp, Stmt};
use crate::diag::{Diagnostic, Pos};
use std::collections::BTreeMap;

fn unsupported(pos: Pos, msg: &'static str) -> Diagnostic {
    Diagnostic::new(pos, "E_HANDLE", msg)
}

pub(super) fn has_checked_operation(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Bind { init, .. } => matches!(&init.kind,
            ExprKind::Ownership(OwnershipOp::Ref | OwnershipOp::Deref | OwnershipOp::Unref, _, _)),
        Stmt::Block(b) => has_checked_operation(b),
        Stmt::If { arms, otherwise, .. } =>
            arms.iter().any(|(_, b)| has_checked_operation(b)) ||
            otherwise.as_ref().is_some_and(|b| has_checked_operation(b)),
        Stmt::While { body, .. } | Stmt::Loop { body, .. } |
        Stmt::ForIn { body, .. } => has_checked_operation(body),
        Stmt::For { body, .. } => has_checked_operation(body),
        _ => false,
    })
}

struct Lower<'a> {
    blocks: Vec<Block>,
    handles: BTreeMap<&'a str, u32>,
    next_id: u32,
    pos: Pos,
}

impl<'a> Lower<'a> {
    fn new(pos: Pos) -> Self {
        Self { blocks: Vec::new(), handles: BTreeMap::new(),
            next_id: 1, pos }
    }

    fn id(&mut self) -> Result<u32, Diagnostic> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)
            .ok_or_else(|| unsupported(self.pos, "checked-handle proof identity budget exhausted"))?;
        Ok(id)
    }

    fn node(&mut self, op: Action, tails: &[usize]) -> Result<usize, Diagnostic> {
        if self.blocks.len() >= MAX_CFG_NODES {
            return Err(unsupported(self.pos, "checked-handle CFG node budget exhausted"));
        }
        let idx = self.blocks.len();
        self.blocks.push(Block { action: op, successors: Vec::new() });
        for &prev in tails {
            self.blocks[prev].successors.push(idx);
        }
        Ok(idx)
    }

    /// One lexical scope: each direct deref! lease ends on *all* normal
    /// scope exits, while an early exit is checked before any fake cleanup.
    fn scope(&mut self, body: &'a [Stmt], entry: Vec<usize>, depth: usize)
        -> Result<Vec<usize>, Diagnostic> {
        if depth > 256 {
            return Err(unsupported(self.pos, "checked-handle CFG lexical depth exceeded"));
        }
        let mut tails = entry;
        let mut scoped_views = Vec::new();
        for stmt in body {
            if tails.is_empty() { break; } // dead text after an unconditional exit
            match stmt {
                Stmt::Bind { name, init, .. } => match &init.kind {
                    ExprKind::Ownership(OwnershipOp::Ref, false, _) => {
                        // The type checker has already required a direct
                        // top-level val initializer and unique class owner.
                        let id = self.id()?;
                        if self.handles.insert(name, id).is_some() {
                            return Err(unsupported(self.pos, "duplicate checked-handle CFG identity"));
                        }
                        tails = vec![self.node(Action::Ref { handle: id }, &tails)?];
                    }
                    ExprKind::Ownership(OwnershipOp::Deref, false, from) => {
                        let ExprKind::Var(handle) = &from.kind else {
                            return Err(unsupported(self.pos, "checked deref lost its typed handle binding"));
                        };
                        let id = *self.handles.get(handle.as_str()).ok_or_else(||
                            unsupported(self.pos, "checked deref refers to an unknown handle"))?;
                        let view = self.id()?;
                        tails = vec![self.node(Action::Deref { handle: id, view }, &tails)?];
                        scoped_views.push(view);
                    }
                    ExprKind::Ownership(OwnershipOp::Unref, false, from) => {
                        let ExprKind::Var(handle) = &from.kind else {
                            return Err(unsupported(self.pos, "checked unref lost its typed handle binding"));
                        };
                        let id = *self.handles.get(handle.as_str()).ok_or_else(||
                            unsupported(self.pos, "checked unref refers to an unknown handle"))?;
                        tails = vec![self.node(Action::Unref { handle: id }, &tails)?];
                    }
                    _ => {
                        tails = vec![self.node(Action::Nop, &tails)?];
                    }
                },
                Stmt::Block(inner) => {
                    tails = self.scope(inner, tails, depth + 1)?;
                }
                Stmt::If { arms, otherwise, .. } => {
                    // A real nondeterministic branch in the proof graph.
                    // The typed checker separately authenticates condition
                    // types, effect safety and ownership preservation.
                    let decision = self.node(Action::Nop, &tails)?;
                    let mut merged = Vec::new();
                    for (_, arm) in arms {
                        let arm_entry = self.node(Action::Nop, &[decision])?;
                        merged.extend(self.scope(arm, vec![arm_entry], depth + 1)?);
                    }
                    if let Some(other) = otherwise {
                        let arm_entry = self.node(Action::Nop, &[decision])?;
                        merged.extend(self.scope(other, vec![arm_entry], depth + 1)?);
                    } else {
                        merged.push(self.node(Action::Nop, &[decision])?);
                    }
                    tails = merged;
                }
                Stmt::Return(..) => {
                    let _ = self.node(Action::Exit, &tails)?;
                    tails.clear();
                }
                // In the initial compiler profile, every other construct
                // with nested checked operations is rejected. Loops, try,
                // jumps and failure effects with a live handle are already
                // forbidden during type checking. We never interpret these
                // unknown constructs as verified cleanup.
                other if has_checked_operation(std::slice::from_ref(other)) => {
                    return Err(unsupported(self.pos, "unsupported checked-handle control-flow shape"));
                }
                _ => {
                    tails = vec![self.node(Action::Nop, &tails)?];
                }
            }
        }
        for view in scoped_views.into_iter().rev() {
            if tails.is_empty() { break; }
            tails = vec![self.node(Action::EndView { view }, &tails)?];
        }
        Ok(tails)
    }
}

pub(super) fn verify_type_admitted_body(body: &[Stmt], pos: Pos) -> Result<(), Diagnostic> {
    if !has_checked_operation(body) {
        return Ok(());
    }
    let mut lower = Lower::new(pos);
    let entry = lower.node(Action::Nop, &[])?;
    let tails = lower.scope(body, vec![entry], 0)?;
    if !tails.is_empty() {
        let _ = lower.node(Action::Exit, &tails)?;
    }
    proof::verify_all_paths(&lower.blocks, entry).map_err(|e| {
        let message = match e {
            FlowError::UnreleasedHandle => "a checked handle or view remains live on an exit path",
            FlowError::LiveViewOnUnref => "unref! is reachable while a deref! view is live",
            FlowError::UseAfterUnref => "checked-handle operation after consumption",
            FlowError::InvalidView => "invalid or doubly retired checked view",
            FlowError::DuplicateRef => "duplicate checked handle identity",
            FlowError::UnsafeCall => "unproven call while a checked handle is live",
            FlowError::UnsafeSuspend => "unproven suspend while a checked handle is live",
            FlowError::LiveCycle => "checked handle retained across an unproven CFG cycle",
            FlowError::Budget => "checked-handle CFG proof exceeded its bounded budget",
            _ => "checked-handle CFG proof failed closed",
        };
        Diagnostic::new(pos, "E_HANDLE", message)
    })
}
