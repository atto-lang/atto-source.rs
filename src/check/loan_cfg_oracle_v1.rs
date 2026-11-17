//! Bounded, independently recomputed MAY-loan liveness over checked typed-IR
//! control-flow for the *restricted* scalar branch profile.
//!
//! This module does not trust ScalarLoanEnd or ScalarLoanScopeEnd as a
//! statement of future-use deadness. It rewalks typed expressions, builds
//! actual if/else CFG edges and solves backwards liveness without NLL ends.
//! It also forwards checks MAY-active conflicts and MUST-active use/end sites.
//!
//! Diagnostic only. Facts and emitted ops still originate from the same
//! Atto compiler; this is not cross-compiler Polonius or codegen authority.
//! Loops, exceptional, async, actor, heap, FFI and unmodeled effects reject.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use crate::ast::Type;
use crate::check::{TExpr, TProgram, TStmt, TE};
use crate::diag::{Diagnostic, Pos};

const MAX_FUNCTIONS: usize = 256;
const MAX_POINTS: usize = 1024;
const MAX_LOANS: usize = 256;
const MAX_EXPR_NODES: usize = 16_384;
const MAX_EXPR_DEPTH: usize = 96;
const MAX_STEPS: usize = 65_536;

fn incomplete(msg: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_LOAN_CFG_INCOMPLETE", msg)
}
fn unsupported(msg: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_LOAN_CFG_UNSUPPORTED", msg)
}
fn budget(msg: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_LOAN_CFG_BUDGET", msg)
}

#[derive(Clone, Debug, Default)]
struct Reads {
    loans: Vec<(u32, u32)>,
    direct: BTreeSet<u32>,
    may_trap: bool,
    conditional: bool,
}
fn scalar(ty: Type) -> bool {
    matches!(ty, Type::Int | Type::Float | Type::Bool)
}
fn reads(expr: &TE, facts: &mut Reads, work: &mut usize, depth: usize, slots: usize) -> Result<(), Diagnostic> {
    *work = work.checked_add(1).ok_or_else(|| budget("typed expression work overflow"))?;
    if *work > MAX_EXPR_NODES || depth > MAX_EXPR_DEPTH {
        return Err(budget("typed expression depth/work exceeded"));
    }
    if !scalar(expr.ty) {
        return Err(unsupported("non-scalar typed expression requires independent effects proof"));
    }
    match &expr.e {
        TExpr::I64(_) | TExpr::F64(_) | TExpr::Bool(_) => {}
        TExpr::Local(slot) => {
            if *slot as usize >= slots {
                return Err(incomplete("direct read has unknown scalar slot"));
            }
            facts.direct.insert(*slot);
        }
        TExpr::LoanRead { loan_id, owner_slot } => {
            if *loan_id == 0 || *owner_slot as usize >= slots {
                return Err(incomplete("borrow read has unknown loan or owner slot"));
            }
            facts.loans.push((*loan_id, *owner_slot));
        }
        TExpr::Not(inner) | TExpr::IntToFloat(inner) => reads(inner, facts, work, depth + 1, slots)?,
        TExpr::Neg(inner) | TExpr::FNeg(inner) => {
            facts.may_trap = true;
            reads(inner, facts, work, depth + 1, slots)?;
        }
        TExpr::Bin(_, left, right) => {
            // Binary operations may use checked arithmetic. Until we have
            // verified unwind/cleanup CFG, conservatively mark ALL of them.
            facts.may_trap = true;
            reads(left, facts, work, depth + 1, slots)?;
            reads(right, facts, work, depth + 1, slots)?;
        }
        TExpr::Builtin(_, left, right) => {
            reads(left, facts, work, depth + 1, slots)?;
            reads(right, facts, work, depth + 1, slots)?;
        }
        TExpr::Ternary(c, a, b) => {
            facts.conditional = true;
            reads(c, facts, work, depth + 1, slots)?;
            reads(a, facts, work, depth + 1, slots)?;
            reads(b, facts, work, depth + 1, slots)?;
        }
        _ => return Err(unsupported("unmodeled expression effect/call/alias in loan CFG")),
    }
    Ok(())
}
fn collect(expr: Option<&TE>, work: &mut usize, slots: usize) -> Result<Reads, Diagnostic> {
    let mut facts = Reads::default();
    if let Some(expr) = expr {
        reads(expr, &mut facts, work, 0, slots)?;
    }
    Ok(facts)
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Issue { loan_id: u32, owner_slot: u32, exclusive: bool },
    /// A marker, not a codegen load. Must be reconciled independently.
    Marker { loan_id: u32, owner_slot: u32, write: bool },
    End { loan_id: u32, owner_slot: u32, lexical: bool },
    Set { owner_slot: u32, via: Option<u32> },
    Eval,
    Condition,
    Return,
}
#[derive(Clone, Debug)]
pub struct Point {
    pub id: u32,
    pub successors: Vec<u32>,
    pub kind: Kind,
    pub borrowed_reads: Vec<(u32, u32)>,
    pub direct_reads: Vec<u32>,
    pub may_trap: bool,
    pub conditional: bool,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub entry: u32,
    pub points: Vec<Point>,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub contract: &'static str,
    pub functions: Vec<Function>,
}
pub const CONTRACT: &str = "atto-typed-loan-cfg-oracle-v1";

struct Build {
    nodes: Vec<Point>,
    expr_work: usize,
    slots: usize,
}
impl Build {
    fn add(&mut self, kind: Kind, facts: Reads, mut successors: Vec<usize>) -> Result<usize, Diagnostic> {
        if self.nodes.len() >= MAX_POINTS {
            return Err(budget("typed CFG point budget exceeded"));
        }
        successors.sort_unstable();
        successors.dedup();
        let id = self.nodes.len();
        self.nodes.push(Point {
            id: id as u32,
            successors: successors.into_iter().map(|x| x as u32).collect(),
            kind,
            borrowed_reads: facts.loans,
            direct_reads: facts.direct.into_iter().collect(),
            may_trap: facts.may_trap,
            conditional: facts.conditional,
        });
        Ok(id)
    }
    fn seq(&mut self, body: &[TStmt], mut next: Option<usize>, depth: usize) -> Result<Option<usize>, Diagnostic> {
        if depth > 64 {
            return Err(budget("typed CFG nesting depth exceeded"));
        }
        for (stmt_index, stmt) in body.iter().enumerate().rev() {
            let successors = next.into_iter().collect::<Vec<_>>();
            let entry = match stmt {
                TStmt::ScalarBorrowIssue { loan_id, owner_slot, exclusive, .. } =>
                    self.add(Kind::Issue { loan_id: *loan_id, owner_slot: *owner_slot, exclusive: *exclusive }, Reads::default(), successors)?,
                TStmt::ScalarLoanUse { loan_id, owner_slot, write } =>
                    self.add(Kind::Marker { loan_id: *loan_id, owner_slot: *owner_slot, write: *write }, Reads::default(), successors)?,
                TStmt::ScalarLoanEnd { loan_id, owner_slot } =>
                    self.add(Kind::End { loan_id: *loan_id, owner_slot: *owner_slot, lexical: false }, Reads::default(), successors)?,
                TStmt::ScalarLoanScopeEnd { loan_id, owner_slot } =>
                    self.add(Kind::End { loan_id: *loan_id, owner_slot: *owner_slot, lexical: true }, Reads::default(), successors)?,
                TStmt::Set(slot, value) => {
                    if *slot as usize >= self.slots {
                        return Err(incomplete("typed scalar set targets unknown slot"));
                    }
                    let reads = collect(Some(value), &mut self.expr_work, self.slots)?;
                    // Checked exclusive alias writes end with a write-use
                    // marker immediately before their actual local.set.
                    // Reconciliation independently verifies the pair.
                    let via = stmt_index.checked_sub(1)
                        .and_then(|i| body.get(i))
                        .and_then(|previous| match previous {
                            TStmt::ScalarLoanUse { loan_id, owner_slot, write: true }
                                if *owner_slot == *slot => Some(*loan_id),
                            _ => None,
                        });
                    self.add(Kind::Set { owner_slot: *slot, via }, reads, successors)?
                }
                TStmt::Expr(value) => {
                    let reads = collect(Some(value), &mut self.expr_work, self.slots)?;
                    self.add(Kind::Eval, reads, successors)?
                }
                TStmt::Return(value) => {
                    let reads = collect(value.as_ref(), &mut self.expr_work, self.slots)?;
                    self.add(Kind::Return, reads, Vec::new())?
                }
                TStmt::Block(children) if !children.is_empty() => {
                    next = self.seq(children, next, depth + 1)?;
                    continue;
                }
                TStmt::If(arms, otherwise) if !arms.is_empty() && arms.len() <= 4 => {
                    let mut false_path = match otherwise {
                        Some(other) if !other.is_empty() => self.seq(other, next, depth + 1)?,
                        Some(_) | None => next,
                    };
                    for (condition, then_body) in arms.iter().rev() {
                        let facts = collect(Some(condition), &mut self.expr_work, self.slots)?;
                        if !facts.loans.is_empty() || facts.conditional {
                            return Err(unsupported("loan reads/ternary in if guards require guard-event CFG"));
                        }
                        let true_path = self.seq(then_body, next, depth + 1)?.or(next);
                        let branches = [true_path, false_path].into_iter().flatten().collect();
                        let id = self.add(Kind::Condition, facts, branches)?;
                        false_path = Some(id);
                    }
                    false_path.expect("nonempty if arms have condition")
                }
                _ => return Err(unsupported("loop, abrupt exit, opaque scope or effect requires more CFG provenance")),
            };
            next = Some(entry);
        }
        Ok(next)
    }
}

// Validate source-checker event envelopes *separately* from future-use
// liveness. A marker must accompany exactly one actual typed load/store.
fn reconcile_sequence(body: &[TStmt], work: &mut usize, slots: usize, depth: usize) -> Result<(), Diagnostic> {
    if depth > 64 {
        return Err(budget("loan marker nesting depth exceeded"));
    }
    let mut reads_expected = Vec::<(u32, u32)>::new();
    let mut write_expected: Option<(u32, u32)> = None;
    for stmt in body {
        match stmt {
            TStmt::ScalarLoanUse { loan_id, owner_slot, write } => {
                if *write {
                    if write_expected.replace((*loan_id, *owner_slot)).is_some() {
                        return Err(incomplete("multiple unmatched alias write markers"));
                    }
                } else {
                    reads_expected.push((*loan_id, *owner_slot));
                }
            }
            TStmt::Set(_, expr) | TStmt::Expr(expr) => {
                let observed = collect(Some(expr), work, slots)?;
                if observed.loans != reads_expected {
                    return Err(incomplete("typed loan reads differ from preceding source-checker markers"));
                }
                reads_expected.clear();
                match stmt {
                    TStmt::Set(slot, _) => {
                        if write_expected.is_some_and(|(_, owner)| owner != *slot) {
                            return Err(incomplete("alias write targeted another scalar slot"));
                        }
                        write_expected = None;
                    }
                    _ if write_expected.is_some() =>
                        return Err(incomplete("alias write marker precedes non-store")),
                    _ => {}
                }
            }
            TStmt::Return(expr) => {
                let observed = collect(expr.as_ref(), work, slots)?;
                if observed.loans != reads_expected || write_expected.is_some() {
                    return Err(incomplete("unmatched borrowed terminal expression"));
                }
                reads_expected.clear();
            }
            TStmt::If(arms, otherwise) if !arms.is_empty() && arms.len() <= 4 => {
                if !reads_expected.is_empty() || write_expected.is_some() {
                    return Err(unsupported("loan markers on conditional guards need event CFG"));
                }
                for (_, b) in arms {
                    reconcile_sequence(b, work, slots, depth + 1)?;
                }
                if let Some(b) = otherwise {
                    reconcile_sequence(b, work, slots, depth + 1)?;
                }
            }
            TStmt::Block(b) if !b.is_empty() => {
                if !reads_expected.is_empty() || write_expected.is_some() {
                    return Err(incomplete("loan marker escaped a checked lexical block"));
                }
                reconcile_sequence(b, work, slots, depth + 1)?;
            }
            TStmt::ScalarBorrowIssue { .. } | TStmt::ScalarLoanEnd { .. } | TStmt::ScalarLoanScopeEnd { .. } => {
                if !reads_expected.is_empty() || write_expected.is_some() {
                    return Err(incomplete("loan issue/end before a pending alias guest operation"));
                }
            }
            _ => return Err(unsupported("loan event reconciliation requires closed CFG profile")),
        }
        if reads_expected.len() > MAX_EXPR_NODES {
            return Err(budget("pending alias event budget exceeded"));
        }
    }
    if !reads_expected.is_empty() || write_expected.is_some() {
        return Err(incomplete("unconsumed scalar loan use markers at scope end"));
    }
    Ok(())
}

fn audit_function(name: &str, body: &[TStmt], slots: usize) -> Result<Function, Diagnostic> {
    let mut maker = Build { nodes: Vec::new(), expr_work: 0, slots };
    reconcile_sequence(body, &mut maker.expr_work, slots, 0)?;
    let entry = maker.seq(body, None, 0)?.ok_or_else(|| unsupported("empty typed function CFG"))?;
    let nodes = &maker.nodes;
    let n = nodes.len();
    let mut seen = vec![false; n];
    let mut queue = VecDeque::from([entry]);
    while let Some(id) = queue.pop_front() {
        if seen[id] { continue; }
        seen[id] = true;
        for &next in &nodes[id].successors { queue.push_back(next as usize); }
    }
    if seen.iter().any(|is_seen| !is_seen) {
        return Err(incomplete("typed CFG contains unconnected/unreachable operation"));
    }

    // Canonical identity table is extracted from typechecked issue nodes.
    // No markers are taken as authoritative origin facts.
    let mut loan_owner = BTreeMap::<u32, (u32, bool)>::new();
    for node in nodes {
        if let Kind::Issue { loan_id, owner_slot, exclusive } = node.kind {
            if loan_id == 0 || owner_slot as usize >= slots
                || loan_owner.insert(loan_id, (owner_slot, exclusive)).is_some()
            {
                return Err(incomplete("non-unique or invalid typed loan issue identity"));
            }
        }
    }
    if loan_owner.len() > MAX_LOANS {
        return Err(budget("issued loan identity budget exceeded"));
    }
    // Backwards MAY future-use fixed point; Ends DO NOT kill future liveness.
    // Only a matching Issue separates pre-issuance from later use.
    let mut future_in = vec![BTreeSet::<u32>::new(); n];
    let mut steps = 0;
    loop {
        let mut changed = false;
        for idx in 0..n {
            steps += 1;
            if steps > MAX_STEPS {
                return Err(budget("CFG future-use fixed point budget exceeded"));
            }
            let node = &nodes[idx];
            let mut live = BTreeSet::new();
            for &to in &node.successors { live.extend(future_in[to as usize].iter().copied()); }
            for &(loan, owner) in &node.borrowed_reads {
                if loan_owner.get(&loan).is_none_or(|(slot, _)| *slot != owner) {
                    return Err(incomplete("typed loan load lacks verified owner/issue identity"));
                }
                live.insert(loan);
            }
            if let Kind::Issue { loan_id, .. } = node.kind { live.remove(&loan_id); }
            if live != future_in[idx] { future_in[idx] = live; changed = true; }
        }
        if !changed { break; }
    }
    for node in nodes {
        if let Kind::End { loan_id, owner_slot, .. } = node.kind {
            if loan_owner.get(&loan_id).is_none_or(|(owner, _)| *owner != owner_slot) {
                return Err(incomplete("loan end provenance disagrees with issued owner"));
            }
            if node.successors.iter().any(|to| future_in[*to as usize].contains(&loan_id)) {
                return Err(incomplete("loan ended while a later reachable typed alias use remains"));
            }
        }
    }

    // Forward MAY active-loan propagation. A join must have an identical
    // active set from all of its reachable incoming paths; otherwise reject
    // rather than assume a loan was issued or ended on both paths.
    let mut inbound = vec![None::<BTreeSet<u32>>; n];
    inbound[entry] = Some(BTreeSet::new());
    let mut worklist = VecDeque::from([entry]);
    let mut transitions = 0usize;
    while let Some(id) = worklist.pop_front() {
        transitions += 1;
        if transitions > MAX_STEPS {
            return Err(budget("CFG active-loan fixed point budget exceeded"));
        }
        let node = &nodes[id];
        let mut active = inbound[id].clone().expect("queued reachable CFG state");
        if (node.may_trap || node.conditional) && !active.is_empty() {
            return Err(unsupported("live loan crosses unchecked trap or conditional expression"));
        }
        for &slot in &node.direct_reads {
            if active.iter().any(|loan| loan_owner.get(loan).is_some_and(|(owner, exclusive)| *owner == slot && *exclusive)) {
                return Err(incomplete("direct owner read while exclusive loan is active"));
            }
        }
        for &(loan, slot) in &node.borrowed_reads {
            if !active.contains(&loan) || loan_owner.get(&loan).is_none_or(|(owner, _)| *owner != slot) {
                return Err(incomplete("typed alias read outside its active loan interval"));
            }
        }
        match node.kind {
            Kind::Issue { loan_id, owner_slot, exclusive } => {
                if active.contains(&loan_id) || active.iter().any(|loan| loan_owner.get(loan).is_some_and(|(owner, ex)| *owner == owner_slot && (*ex || exclusive))) {
                    return Err(incomplete("overlapping/conflicting loan issued on reachable CFG path"));
                }
                active.insert(loan_id);
            }
            Kind::Marker { loan_id, owner_slot, write } => {
                if !active.contains(&loan_id) || loan_owner.get(&loan_id).is_none_or(|(owner, ex)| *owner != owner_slot || (write && !ex)) {
                    return Err(incomplete("loan marker references unavailable owner/permission"));
                }
            }
            Kind::End { loan_id, owner_slot, .. } => {
                if !active.remove(&loan_id) || loan_owner.get(&loan_id).is_none_or(|(owner, _)| *owner != owner_slot) {
                    return Err(incomplete("loan ended on an unissued or inactive CFG path"));
                }
            }
            Kind::Set { owner_slot, via } => {
                if active.iter().any(|loan| loan_owner.get(loan).is_some_and(|(owner, _)| *owner == owner_slot)) {
                    // The exact write-through marker is checked by the
                    // separate statement reconciliation, but this flat node
                    // needs the verified associated loan to authorize it.
                    if via.is_none_or(|loan_id| !active.contains(&loan_id)
                        || loan_owner.get(&loan_id).is_none_or(|(owner, exclusive)| *owner != owner_slot || !exclusive)
                        || active.iter().any(|other| *other != loan_id && loan_owner.get(other).is_some_and(|(owner, _)| *owner == owner_slot)))
                    {
                        return Err(incomplete("owner write without sole live exclusive loan right"));
                    }
                }
            }
            Kind::Return => {
                if !active.is_empty() {
                    return Err(unsupported("return with active loan needs explicit unwind/lexical close"));
                }
            }
            Kind::Eval | Kind::Condition => {}
        }
        if node.successors.is_empty() && !active.is_empty() {
            return Err(unsupported("CFG exits without closing all scoped scalar loans"));
        }
        for &next in &node.successors {
            let slot = &mut inbound[next as usize];
            match slot {
                Some(existing) if *existing != active =>
                    return Err(unsupported("CFG join has path-dependent loan state; require phi/ownership merge proof")),
                Some(_) => {}
                None => { *slot = Some(active.clone()); worklist.push_back(next as usize); }
            }
        }
    }
    Ok(Function { name: name.to_string(), entry: entry as u32, points: maker.nodes })
}

/// Opt-in typed-MIR CFG oracle for *closed* scalar branches.
/// Rejecting is not a judgment about production AttoLang compilation.
pub fn audit_checked_program(program: &TProgram) -> Result<Report, Diagnostic> {
    if program.funcs.is_empty() || program.funcs.len() > MAX_FUNCTIONS
        || !program.generators.is_empty() || !program.actors.is_empty()
        || program.heap || program.guest_eh
    {
        return Err(unsupported("non-scalar/actor/exception program requires other CFG ownership proof"));
    }
    let mut functions = Vec::new();
    for f in &program.funcs {
        let slots = f.params.len().checked_add(f.locals.len()).ok_or_else(|| budget("scalar slot count overflow"))?;
        if slots > MAX_POINTS || !scalar(f.ret) && f.ret != Type::Void
            || f.params.iter().chain(f.locals.iter()).any(|&ty| !scalar(ty))
        {
            return Err(unsupported("typed CFG requires bounded pure scalar function slots"));
        }
        functions.push(audit_function(&f.name, &f.body, slots)?);
    }
    Ok(Report { contract: CONTRACT, functions })
}
