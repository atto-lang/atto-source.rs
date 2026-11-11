//! Canonical AttoLang borrow checker, **restricted V0**.
//!
//! This module checks a typed ownership-event CFG. It does not parse AttoLang,
//! infer traits from syntax, verify FFI witnesses, or authorize a backend's codegen.
//! The frontend MUST derive owner identities and Copy/Send witnesses independently.
//! This V0 API rejects loops, pin-across-suspension, exceptional exits and
//! divergent cleanup joins. `cyclic_v1` is a separate projection/loop model;
//! it does not silently broaden this API's cleanup or MIR proof contract.
use std::collections::{BTreeMap, BTreeSet};

pub mod actor_state_guard_v1;

pub type OwnerId = u32;
pub type LoanId = u32;
pub type BlockId = u32;
pub type SharedRegionId = u32;
/// Compiler-local identity of one in-flight mailbox handoff, not a runtime grant.
pub type SendTicketId = u32;

const MAX_OWNERS: usize = 64;
const MAX_LOANS: usize = 1024;
const MAX_BLOCKS: usize = 4096;
const MAX_EDGES: usize = 16384;
const MAX_OPERATIONS_PER_BLOCK: usize = 1024;
// An opt-in verifier trace must be bounded independently of the CFG budgets.
const MAX_SHADOW_EVENTS_V0: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Traits {
    /// Must be established by the frontend's nominal/structural type rules.
    pub is_copy: bool,
    /// Moving into a Task requires a separate transitive Send proof.
    pub is_send: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    Shared,
    Mutable,
}

/// All OwnerId operands denote canonical alias roots, not arbitrary field IDs.
/// A field projection must be resolved to a root unless disjointness is proven.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Init {
        owner: OwnerId,
        traits: Traits,
    },
    Read {
        owner: OwnerId,
    },
    Write {
        owner: OwnerId,
    },
    Borrow {
        owner: OwnerId,
        loan: LoanId,
        kind: BorrowKind,
    },
    WriteThrough {
        owner: OwnerId,
        loan: LoanId,
    },
    EndBorrow {
        loan: LoanId,
    },
    Move {
        from: OwnerId,
        into: OwnerId,
    },
    Copy {
        from: OwnerId,
        into: OwnerId,
    },
    Drop {
        owner: OwnerId,
    },
    TaskCaptureCopy {
        owner: OwnerId,
    },
    TaskCaptureMove {
        owner: OwnerId,
    },
    Suspend,
    ActorHopBorrow {
        owner: OwnerId,
    },
    /// Reserve an owned Send payload while a mailbox waits for admission.
    /// Sender access is blocked until the ticket commits or aborts.
    ActorSendPrepare {
        owner: OwnerId,
        ticket: SendTicketId,
    },
    /// Committed mailbox admission consumes the previously prepared sender owner.
    ActorSendCommit {
        ticket: SendTicketId,
    },
    /// Failed/aborted admission returns the prepared owner to the sender.
    ActorSendAbort {
        ticket: SendTicketId,
    },
    /// Successful mailbox admission transfers this owned Send value.
    /// The runtime must commit or atomically roll back a failed send.
    ActorSendMove {
        owner: OwnerId,
    },
    /// Copy + Send only: the sender retains its own independent value.
    ActorSendCopy {
        owner: OwnerId,
    },
    /// Consume the mutable owner before publishing a deeply frozen Sync value.
    /// The compiler must independently prove transitive Sync and freezing.
    PublishFrozen {
        owner: OwnerId,
        region: SharedRegionId,
        sync_proven: bool,
    },
    /// Consume the unique owner into a runtime-controlled guarded region.
    /// This never grants actors raw shared-memory write authority.
    PromoteGuarded {
        owner: OwnerId,
        region: SharedRegionId,
    },
    Return,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: BlockId,
    pub operations: Vec<Op>,
    pub successors: Vec<BlockId>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub entry: BlockId,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub block: BlockId,
    pub operation: usize,
}

impl Diagnostic {
    fn at(code: &'static str, block: BlockId, operation: usize) -> Self {
        Self {
            code,
            block,
            operation,
        }
    }
}

/// Exit owners appear in deterministic reverse acquisition order.
/// This is a *proposed* plan; the compiler must independently prove all
/// destruction, unwinding and runtime callback paths before emitting code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEventV0 {
    pub block: BlockId,
    pub operation: usize,
    pub kind: &'static str,
    pub owner: Option<OwnerId>,
}

fn shadow_kind(op: &Op) -> (&'static str, Option<OwnerId>) {
    match *op {
        Op::Init { owner, .. } => ("INIT", Some(owner)),
        Op::Read { owner } => ("READ", Some(owner)),
        Op::Write { owner } => ("WRITE", Some(owner)),
        Op::Borrow {
            owner,
            kind: BorrowKind::Shared,
            ..
        } => ("BORROW_SHARED", Some(owner)),
        Op::Borrow {
            owner,
            kind: BorrowKind::Mutable,
            ..
        } => ("BORROW_MUT", Some(owner)),
        Op::WriteThrough { owner, .. } => ("WRITE_VIA_MUT", Some(owner)),
        Op::EndBorrow { .. } => ("END_BORROW", None),
        Op::Move { from, .. } => ("MOVE", Some(from)),
        Op::Copy { from, .. } => ("COPY", Some(from)),
        Op::Drop { owner } => ("DROP", Some(owner)),
        Op::TaskCaptureCopy { owner } => ("TASK_COPY", Some(owner)),
        Op::TaskCaptureMove { owner } => ("TASK_MOVE", Some(owner)),
        Op::Suspend => ("SUSPEND_SAFE", None),
        Op::ActorHopBorrow { owner } => ("ACTOR_HOP_BORROW", Some(owner)),
        Op::ActorSendPrepare { owner, .. } => ("ACTOR_SEND_PREPARE", Some(owner)),
        Op::ActorSendCommit { .. } => ("ACTOR_SEND_COMMIT", None),
        Op::ActorSendAbort { .. } => ("ACTOR_SEND_ABORT", None),
        Op::ActorSendMove { owner } => ("ACTOR_SEND_MOVE", Some(owner)),
        Op::ActorSendCopy { owner } => ("ACTOR_SEND_COPY", Some(owner)),
        Op::PublishFrozen { owner, .. } => ("PUBLISH_FROZEN", Some(owner)),
        Op::PromoteGuarded { owner, .. } => ("PROMOTE_GUARDED", Some(owner)),
        Op::Return => ("RETURN", None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub cleanup_by_exit: BTreeMap<BlockId, Vec<OwnerId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
    traits: Traits,
    acquisition: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Loan {
    owner: OwnerId,
    kind: BorrowKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State {
    live: BTreeMap<OwnerId, Owner>,
    ever_owned: BTreeSet<OwnerId>,
    loans: BTreeMap<LoanId, Loan>,
    /// An in-flight send reserves its original semantic owner; no use until settled.
    pending_sends: BTreeMap<SendTicketId, OwnerId>,
    ever_loaned: BTreeSet<LoanId>,
    next_acquisition: u64,
}

fn is_live(s: &State, owner: OwnerId) -> bool {
    s.live.contains_key(&owner)
}

fn has_any_loan(s: &State, owner: OwnerId) -> bool {
    s.loans.values().any(|loan| loan.owner == owner)
}

fn has_mut_loan(s: &State, owner: OwnerId) -> bool {
    s.loans
        .values()
        .any(|loan| loan.owner == owner && loan.kind == BorrowKind::Mutable)
}

fn fresh_destination(s: &State, owner: OwnerId) -> bool {
    !s.ever_owned.contains(&owner) && s.ever_owned.len() < MAX_OWNERS
}

fn next_acquisition(s: &mut State) -> u64 {
    let result = s.next_acquisition;
    s.next_acquisition += 1; // bounded by MAX_OWNERS
    result
}

fn apply(state: &mut State, op: &Op, block: BlockId, index: usize) -> Result<(), Diagnostic> {
    let bad = |code| Diagnostic::at(code, block, index);
    // A pending admission has exclusive escrow of the payload. Every local
    // operation that could read, write, alias, move or re-send it fails closed.
    let accessed_owner = match *op {
        Op::Read { owner }
        | Op::Write { owner }
        | Op::Borrow { owner, .. }
        | Op::WriteThrough { owner, .. }
        | Op::Drop { owner }
        | Op::TaskCaptureCopy { owner }
        | Op::TaskCaptureMove { owner }
        | Op::ActorHopBorrow { owner }
        | Op::ActorSendPrepare { owner, .. }
        | Op::ActorSendMove { owner }
        | Op::ActorSendCopy { owner }
        | Op::PublishFrozen { owner, .. }
        | Op::PromoteGuarded { owner, .. } => Some(owner),
        Op::Move { from, .. } | Op::Copy { from, .. } => Some(from),
        _ => None,
    };
    if let Some(owner) = accessed_owner {
        if state.pending_sends.values().any(|&held| held == owner) {
            return Err(bad("E_ACTOR_SEND_PENDING"));
        }
    }
    match *op {
        Op::Init { owner, traits } => {
            if !fresh_destination(state, owner) {
                return Err(bad("E_OWNER_ID_REUSED_OR_BUDGET"));
            }
            let acquisition = next_acquisition(state);
            state.ever_owned.insert(owner);
            state.live.insert(
                owner,
                Owner {
                    traits,
                    acquisition,
                },
            );
        }
        Op::Read { owner } => {
            if !is_live(state, owner) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            if has_mut_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
        }
        Op::Write { owner } => {
            if !is_live(state, owner) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            if has_any_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
        }
        Op::Borrow { owner, loan, kind } => {
            if !is_live(state, owner) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            if state.ever_loaned.contains(&loan) {
                return Err(bad("E_LOAN_ID_REUSED"));
            }
            if state.ever_loaned.len() >= MAX_LOANS {
                return Err(bad("E_LOAN_BUDGET"));
            }
            if has_mut_loan(state, owner)
                || (kind == BorrowKind::Mutable && has_any_loan(state, owner))
            {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            state.ever_loaned.insert(loan);
            state.loans.insert(loan, Loan { owner, kind });
        }
        Op::WriteThrough { owner, loan } => {
            if !is_live(state, owner) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            if !matches!(state.loans.get(&loan), Some(Loan { owner: o, kind: BorrowKind::Mutable }) if *o == owner)
            {
                return Err(bad("E_MUT_LOAN_REQUIRED"));
            }
        }
        Op::EndBorrow { loan } => {
            if state.loans.remove(&loan).is_none() {
                return Err(bad("E_UNKNOWN_LOAN"));
            }
        }
        Op::Move { from, into } => {
            if !is_live(state, from) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            if has_any_loan(state, from) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            if !fresh_destination(state, into) {
                return Err(bad("E_OWNER_ID_REUSED_OR_BUDGET"));
            }
            let moved = state.live.remove(&from).expect("checked live");
            state.ever_owned.insert(into);
            state.live.insert(into, moved); // preserve original acquisition order
        }
        Op::Copy { from, into } => {
            let source = state
                .live
                .get(&from)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_copy {
                return Err(bad("E_COPY_PROOF_REQUIRED"));
            }
            if has_mut_loan(state, from) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            if !fresh_destination(state, into) {
                return Err(bad("E_OWNER_ID_REUSED_OR_BUDGET"));
            }
            let traits = source.traits;
            let acquisition = next_acquisition(state);
            state.ever_owned.insert(into);
            state.live.insert(
                into,
                Owner {
                    traits,
                    acquisition,
                },
            );
        }
        Op::Drop { owner } => {
            if !is_live(state, owner) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            if has_any_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            state.live.remove(&owner);
        }
        Op::TaskCaptureCopy { owner } => {
            let source = state
                .live
                .get(&owner)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_copy || !source.traits.is_send {
                return Err(bad("E_TASK_CAPTURE_PROOF"));
            }
            if has_mut_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
        }
        Op::TaskCaptureMove { owner } => {
            let source = state
                .live
                .get(&owner)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_send {
                return Err(bad("E_TASK_CAPTURE_PROOF"));
            }
            if has_any_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            state.live.remove(&owner);
        }
        Op::Suspend => {
            // Pinned continuation ownership is a future separate proof profile.
            if !state.loans.is_empty() {
                return Err(bad("E_SUSPEND_WITH_LIVE_LOAN"));
            }
        }
        Op::ActorHopBorrow { owner } => {
            if !is_live(state, owner) {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
            return Err(bad("E_ACTOR_BORROW_ESCAPE"));
        }
        Op::ActorSendPrepare { owner, ticket } => {
            let source = state.live.get(&owner)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_send {
                return Err(bad("E_ACTOR_SEND_PROOF_REQUIRED"));
            }
            if has_any_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            if state.pending_sends.insert(ticket, owner).is_some() {
                return Err(bad("E_SEND_TICKET_REUSED"));
            }
        }
        Op::ActorSendCommit { ticket } => {
            let owner = state.pending_sends.remove(&ticket)
                .ok_or_else(|| bad("E_UNKNOWN_ACTOR_SEND_TICKET"))?;
            // Runtime commit must atomically pair this IR effect with one
            // recipient owner; a model proof alone cannot perform admission.
            if state.live.remove(&owner).is_none() {
                return Err(bad("E_USE_AFTER_MOVE_OR_DROP"));
            }
        }
        Op::ActorSendAbort { ticket } => {
            state.pending_sends.remove(&ticket)
                .ok_or_else(|| bad("E_UNKNOWN_ACTOR_SEND_TICKET"))?;
            // The untouched sender owner is usable again only on this path.
        }
        Op::ActorSendMove { owner } | Op::PromoteGuarded { owner, .. } => {
            let source = state
                .live
                .get(&owner)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_send {
                return Err(bad("E_ACTOR_SEND_PROOF_REQUIRED"));
            }
            if has_any_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            // Exactly one semantic owner remains: the receiver/runtime.
            // A source adapter must ensure this runs only after committed
            // admission, with atomic rollback if an external send fails.
            state.live.remove(&owner);
        }
        Op::ActorSendCopy { owner } => {
            let source = state
                .live
                .get(&owner)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_send || !source.traits.is_copy {
                return Err(bad("E_ACTOR_COPY_SEND_PROOF_REQUIRED"));
            }
            if has_mut_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
        }
        Op::PublishFrozen {
            owner,
            sync_proven,
            ..
        } => {
            let source = state
                .live
                .get(&owner)
                .ok_or_else(|| bad("E_USE_AFTER_MOVE_OR_DROP"))?;
            if !source.traits.is_send || !sync_proven {
                return Err(bad("E_SHARED_FROZEN_PROOF_REQUIRED"));
            }
            if has_any_loan(state, owner) {
                return Err(bad("E_BORROW_CONFLICT"));
            }
            // Publication consumes the local owner. The runtime must create
            // independently generation-fenced read-only shared capabilities.
            state.live.remove(&owner);
        }
        Op::Return => {
            if !state.pending_sends.is_empty() {
                return Err(bad("E_ACTOR_SEND_UNRESOLVED"));
            }
            if !state.loans.is_empty() {
                return Err(bad("E_LOAN_ESCAPE"));
            }
        }
    }
    Ok(())
}

/// Acyclic conservative V0 fixed-provenance CFG verification.
///
/// An independently validated, immutable typed compiler IR is required before
/// invoking this API. Unproven syntax or effects must fail closed in the frontend.
pub fn verify(program: &Program) -> Result<Proof, Diagnostic> {
    verify_internal(program, None)
}

/// An opt-in observation hook integrated into the canonical verification loop.
/// Observations are buffered during the same CFG traversal and emitted only
/// after the complete function has passed verification. This prevents a later
/// rejected block from leaking earlier tentative events. The buffer has an
/// explicit cap, and overflow fails closed in shadow mode only. If a consumer
/// returns an error, no Proof is returned; consumer side effects cannot be
/// rolled back. This is not independent source/AST provenance proof.
pub fn verify_with_shadow_v0(
    program: &Program,
    sink: &mut dyn FnMut(ShadowEventV0) -> Result<(), Diagnostic>,
) -> Result<Proof, Diagnostic> {
    verify_internal(program, Some(sink))
}

fn verify_internal(
    program: &Program,
    sink: Option<&mut dyn FnMut(ShadowEventV0) -> Result<(), Diagnostic>>,
) -> Result<Proof, Diagnostic> {
    let structural = |code| Diagnostic::at(code, program.entry, 0);
    if program.blocks.is_empty() || program.blocks.len() > MAX_BLOCKS {
        return Err(structural("E_CFG_BUDGET"));
    }
    // Check caller-owned vector lengths before scanning operations, owner
    // definitions or edges. An oversized block of repeated reads must not
    // consume unbounded work before reaching its eventual budget rejection.
    let mut preflight_edges = 0usize;
    for block in &program.blocks {
        preflight_edges = preflight_edges
            .checked_add(block.successors.len())
            .ok_or_else(|| structural("E_CFG_BUDGET"))?;
        if block.operations.len() > MAX_OPERATIONS_PER_BLOCK || preflight_edges > MAX_EDGES {
            return Err(structural("E_CFG_BUDGET"));
        }
    }
    let mut blocks = BTreeMap::new();
    for block in &program.blocks {
        if blocks.insert(block.id, block).is_some() {
            return Err(structural("E_CFG_DUPLICATE_BLOCK"));
        }
    }
    if !blocks.contains_key(&program.entry) {
        return Err(structural("E_CFG_ENTRY"));
    }
    // Owner/Loan definitions must be unique across the entire function CFG,
    // not merely unique in each path's local State. In particular, two
    // alternative branches cannot mint the same numeric identity and then
    // accidentally merge into identical state snapshots.
    //
    // Real SSA phi nodes will need separately minted values and a verified
    // merge mapping. This restricted V0 conservatively rejects any ID reuse.
    let mut owner_definitions = BTreeSet::new();
    let mut loan_definitions = BTreeSet::new();
    let mut shared_region_definitions = BTreeSet::new();
    let mut send_ticket_definitions = BTreeSet::new();
    for block in blocks.values() {
        for (index, op) in block.operations.iter().enumerate() {
            let owner = match op {
                Op::Init { owner, .. } => Some(*owner),
                Op::Move { into, .. } | Op::Copy { into, .. } => Some(*into),
                _ => None,
            };
            if let Some(id) = owner {
                if id == 0 {
                    return Err(Diagnostic::at("E_OWNER_ID_ZERO", block.id, index));
                }
                if !owner_definitions.insert(id) {
                    return Err(Diagnostic::at("E_OWNER_DEF_REUSED", block.id, index));
                }
                if owner_definitions.len() > MAX_OWNERS {
                    return Err(Diagnostic::at("E_OWNER_ID_BUDGET", block.id, index));
                }
            }
            if let Op::Borrow { loan, .. } = op {
                if *loan == 0 {
                    return Err(Diagnostic::at("E_LOAN_ID_ZERO", block.id, index));
                }
                if !loan_definitions.insert(*loan) {
                    return Err(Diagnostic::at("E_LOAN_DEF_REUSED", block.id, index));
                }
                if loan_definitions.len() > MAX_LOANS {
                    return Err(Diagnostic::at("E_LOAN_BUDGET", block.id, index));
                }
            }
            if let Op::ActorSendPrepare { ticket, .. } = op {
                if *ticket == 0 {
                    return Err(Diagnostic::at("E_SEND_TICKET_ID_ZERO", block.id, index));
                }
                if !send_ticket_definitions.insert(*ticket) {
                    return Err(Diagnostic::at("E_SEND_TICKET_DEF_REUSED", block.id, index));
                }
                if send_ticket_definitions.len() > MAX_OWNERS {
                    return Err(Diagnostic::at("E_SEND_TICKET_BUDGET", block.id, index));
                }
            }
            let region = match op {
                Op::PublishFrozen { region, .. } | Op::PromoteGuarded { region, .. } => {
                    Some(*region)
                }
                _ => None,
            };
            if let Some(region) = region {
                // Region numbers are SSA provenance identities, never runtime
                // authorization tokens. Reject reuse even on divergent paths.
                if region == 0 {
                    return Err(Diagnostic::at("E_REGION_ID_ZERO", block.id, index));
                }
                if !shared_region_definitions.insert(region) {
                    return Err(Diagnostic::at("E_REGION_DEF_REUSED", block.id, index));
                }
                if shared_region_definitions.len() > MAX_OWNERS {
                    return Err(Diagnostic::at("E_REGION_BUDGET", block.id, index));
                }
            }
        }
    }
    let mut indegrees = BTreeMap::new();
    for &id in blocks.keys() {
        indegrees.insert(id, 0usize);
    }
    let mut edges = 0usize;
    for block in blocks.values() {
        if block.operations.len() > MAX_OPERATIONS_PER_BLOCK {
            return Err(structural("E_CFG_BUDGET"));
        }
        let mut seen = BTreeSet::new();
        for &next in &block.successors {
            if !blocks.contains_key(&next) || !seen.insert(next) {
                return Err(structural("E_CFG_EDGE"));
            }
            edges += 1;
            if edges > MAX_EDGES {
                return Err(structural("E_CFG_BUDGET"));
            }
            *indegrees.get_mut(&next).expect("validated successor") += 1;
        }
        let mut return_positions = block
            .operations
            .iter()
            .enumerate()
            .filter_map(|(index, op)| matches!(op, Op::Return).then_some(index));
        let returns = return_positions.next();
        if block.successors.is_empty() {
            if returns != Some(block.operations.len().saturating_sub(1))
                || return_positions.next().is_some()
            {
                return Err(Diagnostic::at(
                    "E_IMPLICIT_EXIT",
                    block.id,
                    block.operations.len(),
                ));
            }
        } else if returns.is_some() {
            return Err(Diagnostic::at(
                "E_RETURN_WITH_SUCCESSOR",
                block.id,
                returns.unwrap_or(0),
            ));
        }
    }
    let mut ready: BTreeSet<BlockId> = indegrees
        .iter()
        .filter_map(|(&id, &count)| if count == 0 { Some(id) } else { None })
        .collect();
    let mut ordering = Vec::new();
    while let Some(&id) = ready.iter().next() {
        ready.remove(&id);
        ordering.push(id);
        for next in &blocks[&id].successors {
            let degree = indegrees.get_mut(next).expect("validated successor");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(*next);
            }
        }
    }
    if ordering.len() != blocks.len() {
        return Err(structural("E_CFG_CYCLE_UNPROVEN"));
    }
    let mut incoming = BTreeMap::<BlockId, Vec<State>>::new();
    incoming.insert(program.entry, vec![State::default()]);
    // None on the ordinary fast path: no trace allocation or second traversal.
    let mut staged_events = sink.as_ref().map(|_| Vec::<ShadowEventV0>::new());
    let mut cleanup_by_exit = BTreeMap::new();
    for id in ordering {
        let paths = incoming
            .remove(&id)
            .ok_or_else(|| Diagnostic::at("E_CFG_UNREACHABLE", id, 0))?;
        let first = paths.first().expect("nonempty incoming");
        if paths.iter().any(|p| p != first) {
            return Err(Diagnostic::at("E_CFG_JOIN_UNPROVEN", id, 0));
        }
        let mut state = first.clone();
        let block = blocks[&id];
        for (index, op) in block.operations.iter().enumerate() {
            apply(&mut state, op, id, index)?;
            if let Some(events) = staged_events.as_mut() {
                if events.len() >= MAX_SHADOW_EVENTS_V0 {
                    return Err(Diagnostic::at("E_SHADOW_EVENT_BUDGET", id, index));
                }
                let (kind, owner) = shadow_kind(op);
                events.push(ShadowEventV0 {
                    block: id,
                    operation: index,
                    kind,
                    owner,
                });
            }
        }
        if block.successors.is_empty() {
            let mut live: Vec<_> = state
                .live
                .iter()
                .map(|(&owner, value)| (value.acquisition, owner))
                .collect();
            live.sort_by(|a, b| b.cmp(a));
            cleanup_by_exit.insert(id, live.into_iter().map(|(_, owner)| owner).collect());
        }
        for &next in &block.successors {
            incoming.entry(next).or_default().push(state.clone());
        }
    }
    let proof = Proof { cleanup_by_exit };
    // Publish only a complete, successfully checked function's event stream.
    // A failing consumer prevents returning Proof; observer side effects are
    // not a substitute for the independently verified proof object.
    if let Some(observer) = sink {
        if let Some(events) = staged_events {
            for event in events {
                observer(event)?;
            }
        }
    }
    Ok(proof)
}

/// Versioned typed-MIR producer envelope for differential compiler adapters.
pub mod mir_v1;

/// Explicit field-projection/loop model; not an implicit V0 or MIR upgrade.
pub mod cyclic_v1;
