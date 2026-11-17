//! Checked-handle all-path verification from *typed compiler IR* slots.
//!
//! The parser/source CFG preflight remains a separate syntax provenance gate.
//! This pass walks TStmt and the actual resolved TFunc locals/parameter types;
//! it cannot be fooled by source variable spelling or shadowing. The normal
//! ownership checker still authenticates that Ref/Unref are direct val binds,
//! that a deref is a read-only view, and that other handles cannot be forged.
//!
//! This is typed IR -> bounded CFG, not yet full effectful MIR: arbitrary loops,
//! exception cleanup, actor/arena transfers and suspension are fail closed.

use super::{any_in, checked_handle_cfg as proof, TExpr, TFunc, TStmt, Type, TE};
use crate::diag::{Diagnostic, Pos};
use proof::{Action, Block, FlowError, MAX_CFG_NODES};

fn invalid(pos: Pos, message: &'static str) -> Diagnostic {
    Diagnostic::new(pos, "E_HANDLE", message)
}

fn can_exit(expr: &TE, may_exit: &[bool]) -> bool {
    any_in(expr, &|node| match &node.e {
        TExpr::Call(target, _) => may_exit.get(*target as usize).copied().unwrap_or(true),
        TExpr::Trap { .. } | TExpr::AdtPropagate { .. } |
        TExpr::Spawn(_) | TExpr::Send { .. } => true,
        _ => false,
    })
}

fn has_checked_mint(body: &[TStmt]) -> bool {
    body.iter().any(|s| match s {
        TStmt::Set(_, value) => matches!(value.ty, Type::CheckedRef(_)),
        TStmt::Block(inner) => has_checked_mint(inner),
        TStmt::If(arms, other) =>
            arms.iter().any(|(_, b)| has_checked_mint(b)) ||
            other.as_ref().is_some_and(|b| has_checked_mint(b)),
        _ => false,
    })
}

struct ProofBuilder<'a> {
    function: &'a TFunc,
    may_exit: &'a [bool],
    nodes: Vec<Block>,
    pos: Pos,
}

impl<'a> ProofBuilder<'a> {
    fn new(function: &'a TFunc, may_exit: &'a [bool], pos: Pos) -> Self {
        Self { function, may_exit, nodes: Vec::new(), pos }
    }

    fn slot_type(&self, slot: u32) -> Option<Type> {
        let i = slot as usize;
        if i < self.function.params.len() {
            self.function.params.get(i).copied()
        } else {
            self.function.locals.get(i - self.function.params.len()).copied()
        }
    }

    fn node(&mut self, action: Action, prev: &[usize]) -> Result<usize, Diagnostic> {
        if self.nodes.len() >= MAX_CFG_NODES {
            return Err(invalid(self.pos, "typed ownership CFG node budget exceeded"));
        }
        let index = self.nodes.len();
        self.nodes.push(Block { action, successors: Vec::new() });
        for &p in prev {
            self.nodes[p].successors.push(index);
        }
        Ok(index)
    }

    fn effect(&mut self, value: &TE, tails: Vec<usize>)
        -> Result<Vec<usize>, Diagnostic> {
        if can_exit(value, self.may_exit) {
            Ok(vec![self.node(Action::CallUnknown, &tails)?])
        } else {
            Ok(tails)
        }
    }

    fn scope(&mut self, stmts: &[TStmt], mut tails: Vec<usize>, depth: usize)
        -> Result<Vec<usize>, Diagnostic> {
        if depth > 256 {
            return Err(invalid(self.pos, "typed ownership CFG nesting budget exceeded"));
        }
        let mut views = Vec::new();
        for stmt in stmts {
            if tails.is_empty() { break; }
            match stmt {
                TStmt::Set(slot, value) => {
                    tails = self.effect(value, tails)?;
                    match value.ty {
                        Type::CheckedRef(c) => {
                            if self.slot_type(*slot) != Some(Type::CheckedRef(c)) || depth != 0 {
                                return Err(invalid(self.pos, "checked handle lost its typed mint provenance"));
                            }
                            tails = vec![self.node(Action::Ref { handle: *slot }, &tails)?];
                        }
                        Type::Class(c) => {
                            // Only the real typed handle slot can be a source
                            // for deref!/unref!: an ordinary class-typed
                            // local never qualifies, even if names shadow.
                            if let TExpr::Local(handle) = value.e {
                                if self.slot_type(handle) == Some(Type::CheckedRef(c)) {
                                    if depth == 0 {
                                        tails = vec![self.node(Action::Unref { handle }, &tails)?];
                                    } else {
                                        if self.slot_type(*slot) != Some(Type::Class(c)) {
                                            return Err(invalid(self.pos, "typed deref view class mismatch"));
                                        }
                                        tails = vec![self.node(Action::Deref {
                                            handle, view: *slot,
                                        }, &tails)?];
                                        views.push(*slot);
                                    }
                                    continue;
                                }
                            }
                            tails = vec![self.node(Action::Nop, &tails)?];
                        }
                        _ => tails = vec![self.node(Action::Nop, &tails)?],
                    }
                }
                TStmt::Block(inner) => tails = self.scope(inner, tails, depth + 1)?,
                TStmt::If(arms, other) => {
                    // Real divergent edges, with each arm's typed slot view
                    // lifetime ending before the eventual join.
                    for (condition, _) in arms {
                        tails = self.effect(condition, tails)?;
                    }
                    let branch = self.node(Action::Nop, &tails)?;
                    let mut out = Vec::new();
                    for (_, stmts) in arms {
                        let arm = self.node(Action::Nop, &[branch])?;
                        out.extend(self.scope(stmts, vec![arm], depth + 1)?);
                    }
                    if let Some(stmts) = other {
                        let arm = self.node(Action::Nop, &[branch])?;
                        out.extend(self.scope(stmts, vec![arm], depth + 1)?);
                    } else {
                        out.push(self.node(Action::Nop, &[branch])?);
                    }
                    tails = out;
                }
                TStmt::Return(value) => {
                    if let Some(value) = value {
                        tails = self.effect(value, tails)?;
                    }
                    self.node(Action::Exit, &tails)?;
                    tails.clear();
                }
                TStmt::Throw(_) | TStmt::Raise(_) | TStmt::Unreachable => {
                    self.node(Action::Exit, &tails)?;
                    tails.clear();
                }
                // Unknown control-flow effects require a separate compiler
                // and runtime cleanup proof before handles can cross them.
                TStmt::Loop { .. } | TStmt::ForIn { .. } |
                TStmt::Try { .. } | TStmt::Defer { .. } |
                TStmt::Recover { .. } | TStmt::RecoverNext |
                TStmt::Break | TStmt::Continue | TStmt::UnwindFrame { .. } => {
                    tails = vec![self.node(Action::Suspend, &tails)?];
                }
                _ => {
                    let unknown = super::any_in_stmts(std::slice::from_ref(stmt), &|e|
                        can_exit(e, self.may_exit));
                    tails = vec![self.node(if unknown {
                        Action::CallUnknown
                    } else {
                        Action::Nop
                    }, &tails)?];
                }
            }
        }
        for view in views.into_iter().rev() {
            if tails.is_empty() { break; }
            tails = vec![self.node(Action::EndView { view }, &tails)?];
        }
        Ok(tails)
    }
}

/// Prove all reachable ownership paths using the typed slots, *after* the
/// independent parser provenance gate and ordinary borrow checker succeeded.
pub(super) fn verify(function: &TFunc, may_exit: &[bool], pos: Pos)
    -> Result<(), Diagnostic> {
    if !has_checked_mint(&function.body) {
        return Ok(());
    }
    let mut proof_builder = ProofBuilder::new(function, may_exit, pos);
    let entry = proof_builder.node(Action::Nop, &[])?;
    let tails = proof_builder.scope(&function.body, vec![entry], 0)?;
    if !tails.is_empty() {
        proof_builder.node(Action::Exit, &tails)?;
    }
    proof::verify_all_paths(&proof_builder.nodes, entry).map_err(|reason| {
        let message = match reason {
            FlowError::UnreleasedHandle => "typed proof found handle or loan alive on exit",
            FlowError::LiveViewOnUnref => "typed proof found unref! with a live view",
            FlowError::UseAfterUnref => "typed proof found use-after-unref",
            FlowError::DuplicateRef => "typed proof found duplicate handle mint",
            FlowError::InvalidView => "typed proof found a stale or invalid read view",
            FlowError::UnsafeCall => "typed proof found an unknown early-exit call under a live handle",
            FlowError::UnsafeSuspend => "typed proof found suspension/control flow under a live handle",
            FlowError::LiveCycle => "typed proof found an unverified live-handle cycle",
            FlowError::Budget => "typed proof exceeded bounded resources",
            _ => "typed checked-handle CFG proof failed closed",
        };
        Diagnostic::new(pos, "E_HANDLE", message)
    })
}
