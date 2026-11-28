//! Opt-in L3 borrow-conformance exporter (`atto-borrow-backend-report-v1`).
//!
//! While the ordinary checker runs, each top-level `fnc` body records the
//! ownership events it decides on, using the canonical vocabulary of
//! `atto-lang/atto-borrow-checker.rs` (`docs/WIRE_FORMAT_V1.md` there). The
//! master checker then re-verifies the exported graph against this compiler's
//! own verdict (`atto-borrow-conform cross-check`).
//!
//! Profile `structured-cfg-v1`: bindings, expressions, blocks, `return`, and
//! structured control flow (`if`/`elif`/`else`, `while`, `loop`, `for`,
//! `break`, `continue`) exported as real CFG blocks. The master checker
//! judges branches and loops with its hybrid V0 + cyclic_v1 verdict.
//! Anything else sets `mirUnavailable` with a reason, never a partial graph:
//! `match`, ternaries, method calls, closures, reassignment, actors,
//! generators, class/actor bodies and so on.
//!
//! Mapping rules (see the conformance plan's M1 notes):
//! - owner id = local slot + 1; loan id = borrowing binder slot + 1;
//!   move/copy destinations start as temporaries and are re-targeted to the
//!   binding when the initializer is exactly `x`, `take! x` or `copy!! x`;
//! - each event is recorded BEFORE the check that may reject it, so a
//!   rejected function exports its prefix up to and including the failing
//!   operation; live lexical loans are then closed and `Return` appended;
//! - `return` (and falling off the end) closes every live lexical loan first.
//!
//! The census (`expectedInstructionIds`) is the recorded event stream itself,
//! NOT an independent typed-IR census; every record says so in `note`.
//! Disabled by default: no cost unless `begin()` was called on this thread.

use std::cell::RefCell;

use crate::diag::{Diagnostic, Pos};

pub const PROFILE: &str = "structured-cfg-v1";
const TEMP_BASE: u32 = 1_000_000;
// The canonical V0 single-block verifier admits at most 1024 operations.
// The final Return and any synthetic EndBorrow operations count as well.
const MAX_TRACE_EVENTS: usize = 1024;
// Whole-function bounds across blocks (well inside canonical V0 budgets).
const MAX_TOTAL_EVENTS: usize = 16_384;
const MAX_BLOCKS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ev {
    Init { owner: u32, copy: bool },
    Read { owner: u32 },
    Borrow { owner: u32, loan: u32, mutable: bool },
    WriteThrough { owner: u32, loan: u32 },
    EndBorrow { loan: u32 },
    Move { from: u32, into: u32 },
    DeepCopy { from: u32, into: u32 },
    Drop { owner: u32 },
    Return,
}

impl Ev {
    /// Compact text form used by tests: `Init(1,copy)`, `Move(1->2)`, ...
    pub fn short(&self) -> String {
        match self {
            Ev::Init { owner, copy } => format!("Init({owner}{})", if *copy { ",copy" } else { "" }),
            Ev::Read { owner } => format!("Read({owner})"),
            Ev::Borrow { owner, loan, mutable } =>
                format!("Borrow({owner},{loan},{})", if *mutable { "mut" } else { "shared" }),
            Ev::WriteThrough { owner, loan } => format!("WriteThrough({owner},{loan})"),
            Ev::EndBorrow { loan } => format!("EndBorrow({loan})"),
            Ev::Move { from, into } => format!("Move({from}->{into})"),
            Ev::DeepCopy { from, into } => format!("DeepCopy({from}->{into})"),
            Ev::Drop { owner } => format!("Drop({owner})"),
            Ev::Return => "Return".to_string(),
        }
    }

    fn json(&self) -> String {
        match self {
            Ev::Init { owner, copy } =>
                format!(r#"{{"op":"Init","owner":{owner},"copy":{copy},"send":false}}"#),
            Ev::Read { owner } => format!(r#"{{"op":"Read","owner":{owner}}}"#),
            Ev::Borrow { owner, loan, mutable } => format!(
                r#"{{"op":"Borrow","owner":{owner},"loan":{loan},"kind":"{}"}}"#,
                if *mutable { "Mutable" } else { "Shared" }),
            Ev::WriteThrough { owner, loan } =>
                format!(r#"{{"op":"WriteThrough","owner":{owner},"loan":{loan}}}"#),
            Ev::EndBorrow { loan } => format!(r#"{{"op":"EndBorrow","loan":{loan}}}"#),
            Ev::Move { from, into } => format!(r#"{{"op":"Move","from":{from},"into":{into}}}"#),
            // The checker only admits copy!! of a flat class after
            // authenticate_flat_copy_aot, so the graph is compiler-proven.
            Ev::DeepCopy { from, into } =>
                format!(r#"{{"op":"DeepCopy","from":{from},"into":{into},"graphProven":true}}"#),
            Ev::Drop { owner } => format!(r#"{{"op":"Drop","owner":{owner}}}"#),
            Ev::Return => r#"{"op":"Return"}"#.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Loan {
    loan: u32,
    owner: u32,
    mutable: bool,
    /// The loan left its lexical scope (returned, aliased, passed on or
    /// created outside a binding): it is never closed, so canonical
    /// verification reports it live at Return (E_LOAN_ESCAPE).
    escaped: bool,
}

/// One exported basic block. Block ids are indices; block 0 is the entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportBlock {
    pub events: Vec<(Ev, Pos)>,
    pub successors: Vec<u32>,
}

/// An exported function graph: real CFG blocks for `if`/loops.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportGraph {
    pub blocks: Vec<ExportBlock>,
}

impl ExportGraph {
    /// Every event in block order (a straight-line graph is one block).
    pub fn iter(&self) -> impl Iterator<Item = &(Ev, Pos)> {
        self.blocks.iter().flat_map(|b| b.events.iter())
    }
    pub fn len(&self) -> usize { self.blocks.iter().map(|b| b.events.len()).sum() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub fn last(&self) -> Option<&(Ev, Pos)> { self.iter().last() }
}

/// An open `if`: the test block whose false edge is still pending, the
/// loans live before the branch, and the arms that fall through.
#[derive(Debug)]
pub struct BranchFrame {
    test: Option<usize>,
    live: Vec<Loan>,
    fallthrough: Vec<usize>,
    must_moved: std::collections::BTreeSet<u32>,
    /// MUST-moved sets of the arms that fall through (intersected at the join).
    arm_must: Vec<std::collections::BTreeSet<u32>>,
}

/// An open loop: header, pending `continue`/`break` sources and the scope
/// depth of its body (locals at or deeper than it are released per iteration).
#[derive(Debug)]
struct LoopFrame {
    header: usize,
    live: Vec<Loan>,
    must_moved: std::collections::BTreeSet<u32>,
    exits: Vec<usize>,
    continues: Vec<usize>,
    body_depth: usize,
}

/// Per-function event recorder, owned by the checker's `FnCx`.
#[derive(Debug)]
pub struct FnTrace {
    blocks: Vec<ExportBlock>,
    /// The block receiving events; `None` once every path has left
    /// (return, break, continue) until control flow rejoins.
    current: Option<usize>,
    total: usize,
    unavailable: Option<String>,
    live: Vec<Loan>,
    /// Owner ids this trace initialized (released at loop-scope ends).
    owners: std::collections::BTreeSet<u32>,
    /// Owners moved or dropped on EVERY path to the current point.
    must_moved: std::collections::BTreeSet<u32>,
    loops: Vec<LoopFrame>,
    next_temp: u32,
    /// (block, index) of the last event with a temporary destination.
    pending_dest: Option<(usize, usize)>,
    /// Skip the next Read of this owner (drop! records its own final read).
    suppress_read: Option<u32>,
}

impl Default for FnTrace {
    fn default() -> Self {
        FnTrace {
            blocks: vec![ExportBlock::default()],
            current: Some(0),
            total: 0,
            unavailable: None,
            live: Vec::new(),
            owners: Default::default(),
            must_moved: Default::default(),
            loops: Vec::new(),
            next_temp: 0,
            pending_dest: None,
            suppress_read: None,
        }
    }
}

pub fn owner_id(slot: u32) -> u32 { slot + 1 }

impl FnTrace {
    pub fn new() -> Self { FnTrace::default() }

    pub fn active(&self) -> bool { self.unavailable.is_none() }

    pub fn unavailable(&mut self, reason: impl Into<String>) {
        if self.unavailable.is_none() {
            self.unavailable = Some(reason.into());
        }
    }

    /// Inside a loop, a move/copy into a temporary that is not bound to a
    /// name (call argument, return value) is consumed by its callee: release
    /// it now, so the next iteration re-initializes a fresh temporary.
    fn settle_pending(&mut self, pos: Pos) {
        let Some((block, index)) = self.pending_dest.take() else { return };
        if self.loops.is_empty() || Some(block) != self.current {
            return;
        }
        if let Ev::Move { into, .. } | Ev::DeepCopy { into, .. } = self.blocks[block].events[index].0 {
            if into > TEMP_BASE {
                self.push_raw(Ev::Drop { owner: into }, pos);
            }
        }
    }

    fn push(&mut self, ev: Ev, pos: Pos) {
        self.settle_pending(pos);
        self.push_raw(ev, pos);
    }

    fn push_raw(&mut self, ev: Ev, pos: Pos) {
        if !self.active() {
            return;
        }
        let Some(block) = self.current else {
            self.unavailable("statement after return");
            return;
        };
        // Canonical V0 admits at most 1024 operations per block (the final
        // Return and synthetic EndBorrow/Drop count) and MAX_TOTAL overall.
        if self.blocks[block].events.len() >= MAX_TRACE_EVENTS || self.total >= MAX_TOTAL_EVENTS {
            self.unavailable("structured-cfg-v1 event budget exceeded");
            return;
        }
        match ev {
            Ev::Init { owner, .. } => {
                self.owners.insert(owner);
                self.must_moved.remove(&owner);
            }
            Ev::Move { from, .. } => { self.must_moved.insert(from); }
            Ev::Drop { owner } => { self.must_moved.insert(owner); }
            _ => {}
        }
        self.blocks[block].events.push((ev, pos));
        self.total += 1;
    }

    fn new_block(&mut self) -> Option<usize> {
        self.settle_pending(Pos::default());
        if self.blocks.len() >= MAX_BLOCKS {
            self.unavailable("structured-cfg-v1 block budget exceeded");
            return None;
        }
        self.blocks.push(ExportBlock::default());
        self.pending_dest = None;
        Some(self.blocks.len() - 1)
    }

    fn edge(&mut self, from: usize, to: usize) {
        self.blocks[from].successors.push(to as u32);
    }

    pub fn init(&mut self, slot: u32, copy: bool, pos: Pos) {
        self.push(Ev::Init { owner: owner_id(slot), copy }, pos);
    }

    /// A read of a named local: an owner, or a lexical loan binder.
    pub fn read(&mut self, slot: u32, pos: Pos) {
        let id = owner_id(slot);
        if self.suppress_read == Some(id) {
            self.suppress_read = None;
            return;
        }
        match self.live.iter().find(|l| l.loan == id).copied() {
            Some(Loan { owner, mutable: false, .. }) => self.push(Ev::Read { owner }, pos),
            // Any access through an exclusive loan is exclusive access.
            Some(Loan { owner, loan, mutable: true, .. }) => self.push(Ev::WriteThrough { owner, loan }, pos),
            None => self.push(Ev::Read { owner: id }, pos),
        }
    }

    fn temp(&mut self) -> u32 {
        self.next_temp += 1;
        TEMP_BASE + self.next_temp
    }

    fn mark_dest(&mut self) {
        if let (true, Some(block)) = (self.active(), self.current) {
            self.pending_dest = Some((block, self.blocks[block].events.len() - 1));
        }
    }

    pub fn moved(&mut self, slot: u32, pos: Pos) {
        if !self.active() {
            return;
        }
        let into = self.temp();
        self.push(Ev::Move { from: owner_id(slot), into }, pos);
        self.mark_dest();
    }

    pub fn deep_copied(&mut self, slot: u32, pos: Pos) {
        if !self.active() {
            return;
        }
        let into = self.temp();
        self.push(Ev::DeepCopy { from: owner_id(slot), into }, pos);
        self.mark_dest();
    }

    /// A new binding. When `retarget` and the immediately preceding event
    /// was a move/copy into a temporary, that value becomes this binding;
    /// otherwise the binding is a freshly initialized owner.
    pub fn bind(&mut self, slot: u32, copy: bool, retarget: bool, pos: Pos) {
        let id = owner_id(slot);
        if retarget {
            if let Some((block, index)) = self.pending_dest {
                if Some(block) == self.current && index + 1 == self.blocks[block].events.len() {
                    match &mut self.blocks[block].events[index].0 {
                        Ev::Move { into, .. } | Ev::DeepCopy { into, .. } => {
                            *into = id;
                            self.pending_dest = None;
                            self.owners.insert(id);
                            self.must_moved.remove(&id);
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }
        self.push(Ev::Init { owner: id, copy }, pos);
    }

    pub fn borrow(&mut self, owner_slot: u32, loan_slot: u32, mutable: bool, pos: Pos) {
        let (owner, loan) = (owner_id(owner_slot), owner_id(loan_slot));
        self.push(Ev::Borrow { owner, loan, mutable }, pos);
        if self.active() {
            self.live.push(Loan { loan, owner, mutable, escaped: false });
            // The checker then evaluates the owner name to initialize the
            // binder: that read IS the loan, not a separate owner access.
            self.suppress_read = Some(owner);
        }
    }

    /// Is this local a live lexical loan binder (not an owner)?
    pub fn is_loan_binder(&self, slot: u32) -> bool {
        let id = owner_id(slot);
        self.live.iter().any(|l| l.loan == id)
    }

    /// A loan binder was transferred (returned, aliased, passed on): the loan
    /// leaves its lexical scope. Records no op; the loan stays live.
    pub fn escape_binder(&mut self, slot: u32) {
        let id = owner_id(slot);
        self.pending_dest = None;
        if let Some(loan) = self.live.iter_mut().find(|l| l.loan == id) {
            loan.escaped = true;
        }
    }

    /// A loan created outside a lexical binding (e.g. `return borrow! x`):
    /// it has no lexical end, so it is born escaped.
    pub fn borrow_escaping(&mut self, owner_slot: u32, mutable: bool, pos: Pos) {
        if !self.active() {
            return;
        }
        let (owner, loan) = (owner_id(owner_slot), self.temp());
        self.push(Ev::Borrow { owner, loan, mutable }, pos);
        if self.active() {
            self.live.push(Loan { loan, owner, mutable, escaped: true });
        }
    }

    /// `val b = borrow! <temporary>`: an unnamed owner, then the loan.
    pub fn borrow_temporary(&mut self, loan_slot: u32, mutable: bool, pos: Pos) {
        if !self.active() {
            return;
        }
        let owner = self.temp();
        self.push(Ev::Init { owner, copy: false }, pos);
        let loan = owner_id(loan_slot);
        self.push(Ev::Borrow { owner, loan, mutable }, pos);
        if self.active() {
            self.live.push(Loan { loan, owner, mutable, escaped: false });
        }
    }

    /// Release lexical locals leaving scope: their loans end and, inside a
    /// loop, still-owned loop-local owners are dropped so the next
    /// iteration's re-initialization is a fresh owner (canonical
    /// `Reinitialize` requires it). `locals` is `(slot, possibly_moved)` in
    /// declaration order, `possibly_moved` from the checker's MAY-moved set.
    fn release(&mut self, locals: &[(u32, bool)], in_loop: bool, pos: Pos) {
        for &(slot, possibly_moved) in locals.iter().rev() {
            let id = owner_id(slot);
            if let Some(index) = self.live.iter().position(|l| l.loan == id) {
                let loan = self.live[index];
                if !loan.escaped {
                    self.push(Ev::EndBorrow { loan: id }, pos);
                }
                continue;
            }
            if in_loop && self.owners.contains(&id) {
                if self.must_moved.contains(&id) {
                    continue; // already consumed on every path: nothing to release
                }
                if possibly_moved {
                    // MAY-moved is not MUST-moved: canonical has no
                    // drop-if-owned operation to release it exactly.
                    self.unavailable("conditionally consumed loop-local owner (needs drop-if-owned)");
                    return;
                }
                self.push(Ev::Drop { owner: id }, pos);
            }
        }
    }

    /// A lexical scope ended (checker `pop_scope`).
    pub fn scope_ended(&mut self, locals: &[(u32, bool)], pos: Pos) {
        if !self.active() {
            return;
        }
        if self.current.is_some() {
            let in_loop = !self.loops.is_empty();
            self.release(locals, in_loop, pos);
        }
        let ended: Vec<u32> = locals.iter().map(|(slot, _)| owner_id(*slot)).collect();
        self.live.retain(|l| !ended.contains(&l.loan));
    }

    /// NLL last-use expiry of lexical loan binders before their scope ends.
    pub fn loans_expired(&mut self, binders: &[u32], pos: Pos) {
        if !self.active() {
            return;
        }
        for &slot in binders {
            let id = owner_id(slot);
            if let Some(index) = self.live.iter().position(|l| l.loan == id) {
                let loan = self.live.remove(index);
                if self.current.is_some() && !loan.escaped {
                    self.push(Ev::EndBorrow { loan: id }, pos);
                }
            }
        }
    }

    pub fn dropped(&mut self, slot: u32, pos: Pos) {
        // The checker evaluates the final primitive read once, after drop!'s
        // checks. Canonically the read precedes the release.
        let id = owner_id(slot);
        self.push(Ev::Read { owner: id }, pos);
        self.push(Ev::Drop { owner: id }, pos);
        self.suppress_read = Some(id);
    }

    /// `return` or falling off the end: lexical loans end, then Return.
    pub fn returned(&mut self, pos: Pos) {
        if self.current.is_none() || !self.active() {
            return;
        }
        let live: Vec<Loan> = self.live.iter().rev().copied().collect();
        for loan in live {
            if !loan.escaped {
                self.push(Ev::EndBorrow { loan: loan.loan }, pos);
            }
        }
        self.push(Ev::Return, pos);
        self.current = None;
    }

    // ---- if / elif / else ---------------------------------------------------

    pub fn branch_begin(&mut self) -> BranchFrame {
        BranchFrame {
            test: None,
            live: self.live.clone(),
            fallthrough: Vec::new(),
            must_moved: self.must_moved.clone(),
            arm_must: Vec::new(),
        }
    }

    /// The arm's condition has been evaluated in the current (test) block.
    pub fn arm_begin(&mut self, frame: &mut BranchFrame) {
        if !self.active() {
            return;
        }
        let Some(test) = self.current else {
            return self.unavailable("statement after return");
        };
        let Some(arm) = self.new_block() else { return };
        self.edge(test, arm);
        frame.test = Some(test);
        self.current = Some(arm);
        self.live = frame.live.clone();
        self.must_moved = frame.must_moved.clone();
    }

    /// The arm's body is done: fall through to the join (if it can), then
    /// continue in the false-edge block for the next condition or `else`.
    pub fn arm_end(&mut self, frame: &mut BranchFrame) {
        if !self.active() {
            return;
        }
        if let Some(arm) = self.current {
            frame.fallthrough.push(arm);
            frame.arm_must.push(self.must_moved.clone());
        }
        let Some(test) = frame.test.take() else { return };
        let Some(next) = self.new_block() else { return };
        self.edge(test, next);
        self.current = Some(next);
        self.live = frame.live.clone();
        self.must_moved = frame.must_moved.clone();
    }

    /// After the `else` body (or the empty false path): join reachable arms.
    pub fn branch_end(&mut self, mut frame: BranchFrame) {
        if !self.active() {
            return;
        }
        if let Some(last) = self.current {
            frame.fallthrough.push(last);
            frame.arm_must.push(self.must_moved.clone());
        }
        self.live = frame.live;
        // MUST-moved at the join: moved on every arm that reaches it.
        let mut sets = frame.arm_must.into_iter();
        self.must_moved = match sets.next() {
            Some(first) => sets.fold(first, |acc, s| acc.intersection(&s).copied().collect()),
            None => frame.must_moved,
        };
        if frame.fallthrough.is_empty() {
            self.current = None;
            return;
        }
        let Some(join) = self.new_block() else { return };
        for from in frame.fallthrough {
            self.edge(from, join);
        }
        self.current = Some(join);
    }

    // ---- while / loop / for -------------------------------------------------

    /// Enter the loop header (the condition, if any, is evaluated there).
    pub fn loop_begin(&mut self, body_depth: usize) {
        if !self.active() {
            return;
        }
        let Some(from) = self.current else {
            return self.unavailable("statement after return");
        };
        let Some(header) = self.new_block() else { return };
        self.edge(from, header);
        self.current = Some(header);
        self.loops.push(LoopFrame {
            header,
            live: self.live.clone(),
            must_moved: self.must_moved.clone(),
            exits: Vec::new(),
            continues: Vec::new(),
            body_depth,
        });
    }

    /// The condition (if any) is done: the header branches into the body,
    /// and to the exit when the loop has a condition.
    pub fn loop_body_begin(&mut self, has_cond: bool) {
        if !self.active() {
            return;
        }
        let (Some(header), Some(frame)) = (self.current, self.loops.last()) else { return };
        let frame_header = frame.header;
        let Some(body) = self.new_block() else { return };
        self.edge(header, body);
        if has_cond {
            if let Some(frame) = self.loops.last_mut() {
                frame.exits.push(header);
            }
        }
        debug_assert_eq!(header, frame_header);
        self.current = Some(body);
    }

    /// `break` / `continue`: release locals of the loop body's scopes still
    /// open at the jump (`locals`, innermost last), then leave the block.
    pub fn loop_jump(&mut self, is_break: bool, locals: &[(u32, bool)], pos: Pos) {
        if !self.active() {
            return;
        }
        let Some(from) = self.current else { return };
        self.release(locals, true, pos);
        if !self.active() {
            return;
        }
        if let Some(frame) = self.loops.last_mut() {
            if is_break { frame.exits.push(from) } else { frame.continues.push(from) }
        }
        self.current = None;
    }

    /// Body done (its scope already popped). For `for`, the update block
    /// runs next; otherwise the back-edges go straight to the header.
    pub fn loop_update_begin(&mut self) -> bool {
        if !self.active() {
            return false;
        }
        let Some(frame) = self.loops.last_mut() else { return false };
        let mut sources = std::mem::take(&mut frame.continues);
        sources.extend(self.current);
        if sources.is_empty() {
            self.current = None;
            return false;
        }
        let Some(update) = self.new_block() else { return false };
        for from in sources {
            self.edge(from, update);
        }
        self.current = Some(update);
        true
    }

    /// Close the loop: back-edges to the header, exit edges to a new block.
    pub fn loop_end(&mut self) {
        if !self.active() {
            return;
        }
        let Some(frame) = self.loops.pop() else { return };
        let mut back = frame.continues;
        back.extend(self.current);
        for from in back {
            self.edge(from, frame.header);
        }
        self.live = frame.live;
        // The checker rejects moving outer values in a loop, so the state
        // after it is the entry state (loop locals are out of scope).
        self.must_moved = frame.must_moved;
        if frame.exits.is_empty() {
            // `loop` without `break`: nothing after it is reachable.
            self.current = None;
            return;
        }
        let Some(exit) = self.new_block() else { return };
        for from in frame.exits {
            self.edge(from, exit);
        }
        self.current = Some(exit);
    }

    /// A rejected function's prefix ends at the failing op. If that op sits
    /// in an open loop, the rejection may depend on repetition (e.g. a
    /// `take!` in a `while` condition), which a truncated prefix would hide.
    /// A failure in the loop *header* is encoded exactly: the header re-runs
    /// (back-edge to itself) or exits. Deeper in an open body, body locals and
    /// loans would need per-iteration release first, so the record fails
    /// closed instead of exporting a misleading single-iteration witness.
    pub fn close_open_loops_for_prefix(&mut self) {
        let Some(frame) = self.loops.last() else { return };
        let (header, live_at_entry) = (frame.header, frame.live.len());
        if self.current != Some(header) || self.live.len() != live_at_entry {
            return self.unavailable("rejected inside a loop body: the prefix cannot encode the back-edge");
        }
        self.loops.pop();
        let Some(exit) = self.new_block() else { return };
        self.edge(header, header);
        self.edge(header, exit);
        self.current = Some(exit);
    }

    /// Depth of the innermost open loop body, if any.
    pub fn loop_body_depth(&self) -> Option<usize> {
        self.loops.last().map(|frame| frame.body_depth)
    }

    pub fn events(&self) -> Vec<Ev> {
        self.blocks.iter().flat_map(|b| b.events.iter().map(|(e, _)| e.clone())).collect()
    }
}

/// One exported function.
#[derive(Clone, Debug)]
pub struct Record {
    pub function: String,
    /// `None` = accepted; `Some((code, message))` = rejected.
    pub local: Option<(String, String)>,
    /// Ok(graph) or Err(reason the graph is unavailable).
    pub mir: Result<ExportGraph, String>,
    pub prefix_at_error: bool,
}

thread_local! {
    static SINK: RefCell<Option<Vec<Record>>> = const { RefCell::new(None) };
}

/// Start collecting records on this thread.
pub fn begin() {
    SINK.with(|s| *s.borrow_mut() = Some(Vec::new()));
}

/// Stop collecting and return the records gathered since `begin()`.
pub fn finish() -> Vec<Record> {
    SINK.with(|s| s.borrow_mut().take()).unwrap_or_default()
}

pub fn enabled() -> bool {
    SINK.with(|s| s.borrow().is_some())
}

/// Called by the checker once per checked body.
pub(crate) fn record(function: &str, trace: Option<FnTrace>, why_untraced: &str,
                     outcome: Option<&Diagnostic>, end: Pos) {
    if !enabled() {
        return;
    }
    let local = outcome.map(|d| (d.code.to_string(), d.message.clone()));
    let (mir, prefix_at_error) = match trace {
        None => (Err(why_untraced.to_string()), false),
        Some(mut t) => {
            if let (Some(d), true) = (outcome, t.active()) {
                if !is_ownership_code(d.code) {
                    t.unavailable(format!("rejected with non-ownership {} before the graph was complete", d.code));
                }
            }
            let prefix = outcome.is_some();
            if prefix && t.active() {
                t.close_open_loops_for_prefix();
            }
            if t.active() {
                t.returned(end);
            }
            match t.unavailable {
                Some(reason) => (Err(reason), false),
                None => (Ok(ExportGraph { blocks: t.blocks }), prefix),
            }
        }
    };
    SINK.with(|s| {
        if let Some(records) = s.borrow_mut().as_mut() {
            records.push(Record { function: function.to_string(), local, mir, prefix_at_error });
        }
    });
}

/// Codes this compiler uses for ownership errors (registry families).
pub fn is_ownership_code(code: &str) -> bool {
    matches!(code, "E_MOVED" | "E_BORROW" | "E_DROPPED")
}

// ---------------------------------------------------------------------------
// atto-borrow-backend-report-v1 output
// ---------------------------------------------------------------------------

pub struct ReportContext<'a> {
    pub compiler_git_sha: &'a str,
    pub source_sha256: &'a str,
    pub source_path: &'a str,
    pub case: Option<&'a str>,
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn mir_json(graph: &ExportGraph, ctx: &ReportContext<'_>, function_id: usize) -> String {
    // Instruction ids are 1.. in block order across the whole function.
    let mut next = 0usize;
    let mut blocks = Vec::with_capacity(graph.blocks.len());
    for (id, block) in graph.blocks.iter().enumerate() {
        let mut instructions = Vec::with_capacity(block.events.len());
        for (ev, pos) in &block.events {
            next += 1;
            let effect = if *ev == Ev::Return { "Control" } else { "Ownership" };
            // Source node id: 1-based line/column packed; always nonzero.
            let node = (u64::from(pos.line.max(1)) << 20) | u64::from(pos.col & 0xFFFFF);
            instructions.push(format!(
                r#"{{"instructionId":{next},"sourceNodeId":{node},"effect":"{effect}","op":{}}}"#,
                ev.json()));
        }
        let successors: Vec<String> = block.successors.iter().map(|s| s.to_string()).collect();
        blocks.push(format!(r#"{{"id":{id},"instructions":[{}],"successors":[{}]}}"#,
                            instructions.join(","), successors.join(",")));
    }
    let ids: Vec<String> = (1..=next).map(|i| i.to_string()).collect();
    format!(
        concat!(r#"{{"version":"atto-borrow-mir-v1","producer":{{"compilerGitSha":{},"sourceSha256":{},"#,
                r#""sourceUnitId":1,"functionId":{}}},"entry":0,"expectedInstructionIds":[{}],"#,
                r#""blocks":[{}]}}"#),
        esc(ctx.compiler_git_sha), esc(ctx.source_sha256), function_id,
        ids.join(","), blocks.join(","))
}

/// One JSON line per record.
pub fn report_lines(records: &[Record], ctx: &ReportContext<'_>) -> String {
    let mut out = String::new();
    for (index, r) in records.iter().enumerate() {
        let local = match &r.local {
            None => r#"{"verdict":"accept","code":null}"#.to_string(),
            Some((code, message)) =>
                format!(r#"{{"verdict":"reject","code":{},"message":{}}}"#, esc(code), esc(message)),
        };
        let mir = match &r.mir {
            Ok(graph) => format!(r#""mir":{}"#, mir_json(graph, ctx, index + 1)),
            Err(reason) => format!(r#""mirUnavailable":{}"#, esc(reason)),
        };
        let note = format!("atto-source.rs borrow_export {PROFILE}; census = event stream (not independent){}",
                           if r.prefix_at_error { "; prefix-at-local-error" } else { "" });
        out.push_str(&format!(
            concat!(r#"{{"schema":"atto-borrow-backend-report-v1","backend":"rs","compilerGitSha":{},"#,
                    r#""sourcePath":{},"case":{},"function":{},"local":{},{},"note":{}}}"#, "\n"),
            esc(ctx.compiler_git_sha), esc(ctx.source_path),
            ctx.case.map_or("null".to_string(), esc), esc(&r.function), local, mir, esc(&note)));
    }
    out
}

#[cfg(test)]
mod trace_budget_tests {
    use super::*;

    #[test]
    fn exact_single_block_budget_includes_return() {
        begin();
        let pos = Pos::default();
        let mut trace = FnTrace::new();
        trace.init(0, true, pos);
        for _ in 0..MAX_TRACE_EVENTS - 2 {
            trace.read(0, pos);
        }
        record("bounded", Some(trace), "", None, pos);
        let records = finish();
        let events = records[0].mir.as_ref().unwrap();
        assert_eq!(events.len(), MAX_TRACE_EVENTS);
        assert_eq!(events.last().unwrap().0, Ev::Return);
    }

    #[test]
    fn budget_exhaustion_never_exports_partial_graph_or_changes_local_verdict() {
        begin();
        let pos = Pos::default();
        let mut trace = FnTrace::new();
        trace.init(0, true, pos);
        for _ in 0..MAX_TRACE_EVENTS {
            trace.read(0, pos);
        }
        // Calls made after admission stops cannot grow the temporary counter.
        trace.moved(0, pos);
        trace.deep_copied(0, pos);
        trace.borrow_escaping(0, false, pos);
        trace.borrow_temporary(1, false, pos);
        assert_eq!(trace.next_temp, 0);
        assert_eq!(trace.total, MAX_TRACE_EVENTS);
        record("overflow", Some(trace), "", None, pos);
        let records = finish();
        assert!(records[0].local.is_none());
        assert!(!records[0].prefix_at_error);
        assert!(matches!(&records[0].mir, Err(reason) if reason.contains("event budget")));
    }

    #[test]
    fn synthetic_loan_cleanup_also_consumes_budget() {
        begin();
        let pos = Pos::default();
        let mut trace = FnTrace::new();
        trace.init(0, true, pos);
        trace.borrow(0, 1, false, pos);
        for _ in 0..MAX_TRACE_EVENTS - 3 {
            trace.read(1, pos);
        }
        // Cleanup fits, but Return does not; no truncated witness is valid.
        record("cleanup", Some(trace), "", None, pos);
        let records = finish();
        assert!(matches!(&records[0].mir, Err(reason) if reason.contains("event budget")));
    }
}
