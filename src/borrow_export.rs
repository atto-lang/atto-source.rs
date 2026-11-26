//! Opt-in L3 borrow-conformance exporter (`atto-borrow-backend-report-v1`).
//!
//! While the ordinary checker runs, each top-level `fnc` body records the
//! ownership events it decides on, using the canonical vocabulary of
//! `atto-lang/atto-borrow-checker.rs` (`docs/WIRE_FORMAT_V1.md` there). The
//! master checker then re-verifies the exported graph against this compiler's
//! own verdict (`atto-borrow-conform cross-check`).
//!
//! Profile `straight-line-v1`. Anything outside it sets `mirUnavailable` with
//! a reason, never a partial graph: control flow, method calls, closures,
//! reassignment, actors, generators, class/actor bodies and so on.
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

pub const PROFILE: &str = "straight-line-v1";
const TEMP_BASE: u32 = 1_000_000;

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

/// Per-function event recorder, owned by the checker's `FnCx`.
#[derive(Debug, Default)]
pub struct FnTrace {
    events: Vec<(Ev, Pos)>,
    unavailable: Option<String>,
    live: Vec<Loan>,
    next_temp: u32,
    /// Index of the last event with a temporary destination.
    pending_dest: Option<usize>,
    /// Skip the next Read of this owner (drop! records its own final read).
    suppress_read: Option<u32>,
    terminated: bool,
    /// Index of the event recorded when the checker rejected (prefix end).
    failed_at: Option<usize>,
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

    fn push(&mut self, ev: Ev, pos: Pos) {
        if !self.active() {
            return;
        }
        if self.terminated {
            self.unavailable("statement after return");
            return;
        }
        self.pending_dest = None;
        self.events.push((ev, pos));
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

    pub fn moved(&mut self, slot: u32, pos: Pos) {
        let into = self.temp();
        self.push(Ev::Move { from: owner_id(slot), into }, pos);
        if self.active() { self.pending_dest = Some(self.events.len() - 1); }
    }

    pub fn deep_copied(&mut self, slot: u32, pos: Pos) {
        let into = self.temp();
        self.push(Ev::DeepCopy { from: owner_id(slot), into }, pos);
        if self.active() { self.pending_dest = Some(self.events.len() - 1); }
    }

    /// A new binding. When `retarget` and the immediately preceding event
    /// was a move/copy into a temporary, that value becomes this binding;
    /// otherwise the binding is a freshly initialized owner.
    pub fn bind(&mut self, slot: u32, copy: bool, retarget: bool, pos: Pos) {
        let id = owner_id(slot);
        if retarget {
            if let Some(index) = self.pending_dest.take() {
                if index + 1 == self.events.len() {
                    match &mut self.events[index].0 {
                        Ev::Move { into, .. } | Ev::DeepCopy { into, .. } => {
                            *into = id;
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
        let (owner, loan) = (owner_id(owner_slot), self.temp());
        self.push(Ev::Borrow { owner, loan, mutable }, pos);
        if self.active() {
            self.live.push(Loan { loan, owner, mutable, escaped: true });
        }
    }

    /// `val b = borrow! <temporary>`: an unnamed owner, then the loan.
    pub fn borrow_temporary(&mut self, loan_slot: u32, mutable: bool, pos: Pos) {
        let owner = self.temp();
        self.push(Ev::Init { owner, copy: false }, pos);
        let loan = owner_id(loan_slot);
        self.push(Ev::Borrow { owner, loan, mutable }, pos);
        if self.active() {
            self.live.push(Loan { loan, owner, mutable, escaped: false });
        }
    }

    /// A lexical scope ended: close loans held by its binders.
    pub fn scope_ended(&mut self, slots: &[u32], pos: Pos) {
        for &slot in slots.iter().rev() {
            let id = owner_id(slot);
            if let Some(index) = self.live.iter().position(|l| l.loan == id) {
                let loan = self.live.remove(index);
                if !self.terminated && !loan.escaped {
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
        if self.terminated {
            return;
        }
        while let Some(loan) = self.live.pop() {
            if !loan.escaped {
                self.push(Ev::EndBorrow { loan: loan.loan }, pos);
            }
        }
        self.push(Ev::Return, pos);
        self.terminated = true;
    }

    pub fn events(&self) -> Vec<Ev> {
        self.events.iter().map(|(e, _)| e.clone()).collect()
    }
}

/// One exported function.
#[derive(Clone, Debug)]
pub struct Record {
    pub function: String,
    /// `None` = accepted; `Some((code, message))` = rejected.
    pub local: Option<(String, String)>,
    /// Ok(events) or Err(reason the graph is unavailable).
    pub mir: Result<Vec<(Ev, Pos)>, String>,
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
            if t.active() {
                t.failed_at = t.events.len().checked_sub(1);
                t.returned(end);
            }
            match t.unavailable {
                Some(reason) => (Err(reason), false),
                None => (Ok(t.events), prefix),
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

fn mir_json(events: &[(Ev, Pos)], ctx: &ReportContext<'_>, function_id: usize) -> String {
    let mut instructions = Vec::with_capacity(events.len());
    for (i, (ev, pos)) in events.iter().enumerate() {
        let effect = if *ev == Ev::Return { "Control" } else { "Ownership" };
        // Source node id: 1-based line/column packed; always nonzero.
        let node = (u64::from(pos.line.max(1)) << 20) | u64::from(pos.col & 0xFFFFF);
        instructions.push(format!(
            r#"{{"instructionId":{},"sourceNodeId":{node},"effect":"{effect}","op":{}}}"#,
            i + 1, ev.json()));
    }
    let ids: Vec<String> = (1..=events.len()).map(|i| i.to_string()).collect();
    format!(
        concat!(r#"{{"version":"atto-borrow-mir-v1","producer":{{"compilerGitSha":{},"sourceSha256":{},"#,
                r#""sourceUnitId":1,"functionId":{}}},"entry":0,"expectedInstructionIds":[{}],"#,
                r#""blocks":[{{"id":0,"instructions":[{}],"successors":[]}}]}}"#),
        esc(ctx.compiler_git_sha), esc(ctx.source_sha256), function_id,
        ids.join(","), instructions.join(","))
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
            Ok(events) => format!(r#""mir":{}"#, mir_json(events, ctx, index + 1)),
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
