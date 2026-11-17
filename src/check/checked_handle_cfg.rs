//! Bounded, path-sensitive checked-handle lifetime oracle.
//!
//! These are explicit compiler-style CFG instructions and proof facts, **not**
//! physical runtime handles or a substitute for typed-MIR integration.
//! Every reachable path must retire all loans before consuming its handle and
//! must consume each minted handle before any Exit. Loops retaining handles
//! at an identical fixed point are denied until a progress/cancellation proof
//! exists. This deliberately rejects some otherwise safe programs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_CFG_NODES: usize = 4096;
pub const MAX_CFG_STATES: usize = 16384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Ref {
        handle: u32,
    },
    Deref {
        handle: u32,
        view: u32,
    },
    EndView {
        view: u32,
    },
    Unref {
        handle: u32,
    },
    /// A normal or exceptional function/actor exit; every live loan must end.
    Exit,
    /// Pure transition with no lifetime effect; branching follows successors.
    Nop,
    /// Unknown effects during a live handle are rejected, not assumed safe.
    CallUnknown,
    /// A suspension/cancellation boundary requires a separate pin proof.
    Suspend,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub action: Action,
    pub successors: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Facts {
    handles: BTreeSet<u32>,
    views: BTreeMap<u32, u32>,
    consumed: BTreeSet<u32>,
}

impl Facts {
    fn fresh() -> Self {
        Self {
            handles: BTreeSet::new(),
            views: BTreeMap::new(),
            consumed: BTreeSet::new(),
        }
    }

    fn has_live(&self) -> bool {
        !self.handles.is_empty() || !self.views.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowError {
    Empty,
    BadEdge,
    Budget,
    DuplicateRef,
    InvalidView,
    UnreleasedHandle,
    LiveViewOnUnref,
    UseAfterUnref,
    UnsafeCall,
    UnsafeSuspend,
    LiveCycle,
    NoExit,
}

/// A conservative all-path proof: every reachable execution must be safe.
///
/// Alternative incoming states at a join are kept separate (not intersected),
/// ensuring that a release in only one arm cannot make another arm appear safe.
/// The state budget prevents adversarial exponential path combinations.
pub fn verify_all_paths(cfg: &[Block], entry: usize) -> Result<(), FlowError> {
    if cfg.is_empty() || entry >= cfg.len() {
        return Err(FlowError::Empty);
    }
    if cfg.len() > MAX_CFG_NODES {
        return Err(FlowError::Budget);
    }
    for block in cfg {
        if block.successors.iter().any(|&n| n >= cfg.len()) {
            return Err(FlowError::BadEdge);
        }
        if matches!(block.action, Action::Exit) {
            if !block.successors.is_empty() {
                return Err(FlowError::BadEdge);
            }
        } else if block.successors.is_empty() {
            // An implicit fallthrough is not an accepted cleanup.
            return Err(FlowError::NoExit);
        }
    }
    let mut pending = VecDeque::from([(entry, Facts::fresh(), BTreeSet::<usize>::new())]);
    let mut seen = BTreeSet::new();
    let mut exits = 0usize;
    while let Some((pc, mut state, ancestors)) = pending.pop_front() {
        // Joins with identical facts are benign, unlike genuine CFG cycles.
        if !seen.insert((pc, state.clone())) {
            continue;
        }
        if seen.len() > MAX_CFG_STATES {
            return Err(FlowError::Budget);
        }
        let block = &cfg[pc];
        match block.action {
            Action::Ref { handle } => {
                if state.handles.contains(&handle) || state.consumed.contains(&handle) {
                    return Err(FlowError::DuplicateRef);
                }
                state.handles.insert(handle);
            }
            Action::Deref { handle, view } => {
                if !state.handles.contains(&handle) {
                    return Err(FlowError::UseAfterUnref);
                }
                if state.views.insert(view, handle).is_some() {
                    return Err(FlowError::InvalidView);
                }
            }
            Action::EndView { view } => {
                if state.views.remove(&view).is_none() {
                    return Err(FlowError::InvalidView);
                }
            }
            Action::Unref { handle } => {
                if !state.handles.contains(&handle) {
                    return Err(FlowError::UseAfterUnref);
                }
                if state.views.values().any(|&owner| owner == handle) {
                    return Err(FlowError::LiveViewOnUnref);
                }
                state.handles.remove(&handle);
                state.consumed.insert(handle);
            }
            Action::Exit => {
                if state.has_live() {
                    return Err(FlowError::UnreleasedHandle);
                }
                exits += 1;
            }
            Action::CallUnknown if state.has_live() => return Err(FlowError::UnsafeCall),
            Action::Suspend if state.has_live() => return Err(FlowError::UnsafeSuspend),
            Action::CallUnknown | Action::Suspend | Action::Nop => {}
        }
        let mut next_ancestors = ancestors;
        next_ancestors.insert(pc);
        for &next in &block.successors {
            if next_ancestors.contains(&next) && state.has_live() {
                return Err(FlowError::LiveCycle);
            }
            pending.push_back((next, state.clone(), next_ancestors.clone()));
        }
    }
    if exits == 0 {
        return Err(FlowError::NoExit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(action: Action, successors: &[usize]) -> Block {
        Block {
            action,
            successors: successors.to_vec(),
        }
    }

    #[test]
    fn canonical_nested_block_proves_all_paths() {
        let cfg = [
            b(Action::Ref { handle: 1 }, &[1]),
            b(Action::Deref { handle: 1, view: 7 }, &[2]),
            b(Action::Nop, &[3, 4]),
            b(Action::Nop, &[5]),
            b(Action::Nop, &[5]),
            b(Action::EndView { view: 7 }, &[6]),
            b(Action::Unref { handle: 1 }, &[7]),
            b(Action::Exit, &[]),
        ];
        assert_eq!(verify_all_paths(&cfg, 0), Ok(()));
    }

    #[test]
    fn a_branch_cannot_hide_missing_unref() {
        let cfg = [
            b(Action::Ref { handle: 1 }, &[1]),
            b(Action::Nop, &[2, 3]),
            b(Action::Unref { handle: 1 }, &[4]),
            b(Action::Nop, &[4]),
            b(Action::Exit, &[]),
        ];
        assert_eq!(verify_all_paths(&cfg, 0), Err(FlowError::UnreleasedHandle));
    }

    #[test]
    fn exceptional_exit_before_unref_is_forbidden() {
        let cfg = [
            b(Action::Ref { handle: 1 }, &[1, 2]),
            b(Action::Unref { handle: 1 }, &[3]),
            b(Action::Exit, &[]),
            b(Action::Exit, &[]),
        ];
        assert_eq!(verify_all_paths(&cfg, 0), Err(FlowError::UnreleasedHandle));
    }

    #[test]
    fn a_view_must_be_retired_before_unref() {
        let cfg = [
            b(Action::Ref { handle: 1 }, &[1]),
            b(Action::Deref { handle: 1, view: 2 }, &[2]),
            b(Action::Unref { handle: 1 }, &[3]),
            b(Action::Exit, &[]),
        ];
        assert_eq!(verify_all_paths(&cfg, 0), Err(FlowError::LiveViewOnUnref));
    }

    #[test]
    fn unsafe_calls_and_suspend_fail_closed() {
        for action in [Action::CallUnknown, Action::Suspend] {
            let cfg = [
                b(Action::Ref { handle: 1 }, &[1]),
                b(action, &[2]),
                b(Action::Unref { handle: 1 }, &[3]),
                b(Action::Exit, &[]),
            ];
            let expected = if matches!(action, Action::CallUnknown) {
                FlowError::UnsafeCall
            } else {
                FlowError::UnsafeSuspend
            };
            assert_eq!(verify_all_paths(&cfg, 0), Err(expected));
        }
    }

    #[test]
    fn loops_with_lingering_handles_need_extra_proof() {
        let cfg = [
            b(Action::Ref { handle: 1 }, &[1]),
            b(Action::Nop, &[1, 2]),
            b(Action::Unref { handle: 1 }, &[3]),
            b(Action::Exit, &[]),
        ];
        assert_eq!(verify_all_paths(&cfg, 0), Err(FlowError::LiveCycle));
    }

    #[test]
    fn no_exit_and_invalid_edges_cannot_be_proved() {
        assert_eq!(verify_all_paths(&[], 0), Err(FlowError::Empty));
        assert_eq!(
            verify_all_paths(&[b(Action::Nop, &[9])], 0),
            Err(FlowError::BadEdge)
        );
        assert_eq!(
            verify_all_paths(&[b(Action::Nop, &[0])], 0),
            Err(FlowError::NoExit)
        );
    }

    #[test]
    fn forged_or_double_lifetime_ops_are_rejected() {
        let cfg = [
            b(Action::Ref { handle: 1 }, &[1]),
            b(Action::Unref { handle: 1 }, &[2]),
            b(Action::Deref { handle: 1, view: 3 }, &[3]),
            b(Action::Exit, &[]),
        ];
        assert_eq!(verify_all_paths(&cfg, 0), Err(FlowError::UseAfterUnref));
    }
}
