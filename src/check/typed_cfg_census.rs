//! Independent, bounded read/definition census over the checked scalar IR.
//! This is diagnostic-only: the current typed IR erases loan issuance and
//! result provenance, so a successful census must never authorize borrowing.
use super::{TExpr, TFunc, TProgram, TStmt, TE};
use crate::ast::Type;
use crate::diag::{Diagnostic, Pos};
use std::collections::{BTreeSet, VecDeque};

const MAX_POINTS: usize = 4096;
const MAX_EXPR_VISITS: usize = 100_000;
const MAX_DEPTH: usize = 128;
const MAX_FUNCTIONS: usize = 256;

fn denied(message: &str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_TYPED_CENSUS", message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointKind {
    Set(u32),
    Eval,
    Branch,
    LoopHead,
    Break,
    Continue,
    Return,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub kind: PointKind,
    pub reads: Vec<u32>,
    pub successors: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCensus {
    pub name: String,
    pub entry: usize,
    pub points: Vec<Point>,
}

fn scalar(ty: Type) -> bool {
    matches!(ty, Type::Int | Type::Bool | Type::Float)
}

struct Builder<'a> {
    function: &'a TFunc,
    points: Vec<Point>,
    remaining_points: &'a mut usize,
    remaining_visits: &'a mut usize,
}

impl Builder<'_> {
    fn slot(&self, slot: u32) -> Result<Type, Diagnostic> {
        let slot = slot as usize;
        self.function.params.iter().chain(&self.function.locals)
            .nth(slot).copied().filter(|ty| scalar(*ty))
            .ok_or_else(|| denied("non-scalar or unknown local slot"))
    }

    fn expr(&mut self, e: &TE, depth: usize, reads: &mut BTreeSet<u32>)
        -> Result<(), Diagnostic>
    {
        if depth >= MAX_DEPTH || *self.remaining_visits == 0 {
            return Err(denied("expression census budget exhausted"));
        }
        *self.remaining_visits -= 1;
        if !scalar(e.ty) {
            return Err(denied("unsupported expression type"));
        }
        match &e.e {
            TExpr::I64(_) | TExpr::Bool(_) | TExpr::F64(_) => {}
            TExpr::Local(slot) => {
                if self.slot(*slot)? != e.ty {
                    return Err(denied("local expression type mismatch"));
                }
                reads.insert(*slot);
            }
            TExpr::Neg(v) | TExpr::Not(v) | TExpr::FNeg(v)
            | TExpr::IntToFloat(v) | TExpr::RefinementCheck(v, _, _, _) => {
                self.expr(v, depth + 1, reads)?;
            }
            TExpr::Bin(_, l, r) | TExpr::Builtin(_, l, r) => {
                self.expr(l, depth + 1, reads)?;
                self.expr(r, depth + 1, reads)?;
            }
            TExpr::Ternary(cond, yes, no) => {
                // May-read census, not evaluation or branch-sensitive liveness.
                self.expr(cond, depth + 1, reads)?;
                self.expr(yes, depth + 1, reads)?;
                self.expr(no, depth + 1, reads)?;
            }
            _ => return Err(denied("unmodelled expression effect or provenance")),
        }
        Ok(())
    }

    fn point(&mut self, kind: PointKind, expr: Option<&TE>, successors: Vec<usize>)
        -> Result<usize, Diagnostic>
    {
        if *self.remaining_points == 0 {
            return Err(denied("point census budget exhausted"));
        }
        *self.remaining_points -= 1;
        let mut reads = BTreeSet::new();
        if let Some(expr) = expr {
            self.expr(expr, 0, &mut reads)?;
        }
        let id = self.points.len();
        self.points.push(Point { kind, reads: reads.into_iter().collect(), successors });
        Ok(id)
    }

    fn block(&mut self, body: &[TStmt], continuation: usize, depth: usize,
        loop_targets: Option<(usize, usize)>)
        -> Result<usize, Diagnostic>
    {
        if depth >= MAX_DEPTH || body.len() > MAX_POINTS {
            return Err(denied("statement census budget exhausted"));
        }
        let mut next = continuation;
        for stmt in body.iter().rev() {
            // Charge even empty structural blocks, not just emitted points.
            if *self.remaining_visits == 0 {
                return Err(denied("statement census budget exhausted"));
            }
            *self.remaining_visits -= 1;
            next = match stmt {
                TStmt::Set(slot, value) => {
                    if self.slot(*slot)? != value.ty {
                        return Err(denied("assignment type mismatch"));
                    }
                    self.point(PointKind::Set(*slot), Some(value), vec![next])?
                }
                TStmt::Expr(value) => self.point(PointKind::Eval, Some(value), vec![next])?,
                TStmt::Return(value) => {
                    if value.as_ref().map_or(Type::Void, |v| v.ty) != self.function.ret {
                        return Err(denied("return type mismatch"));
                    }
                    self.point(PointKind::Return, value.as_ref(), vec![])?
                }
                TStmt::Block(inner) => self.block(inner, next, depth + 1, loop_targets)?,
                TStmt::If(arms, otherwise) => {
                    if arms.is_empty() || arms.len() > MAX_POINTS {
                        return Err(denied("invalid branch census"));
                    }
                    let mut other = match otherwise {
                        Some(body) => self.block(body, next, depth + 1, loop_targets)?,
                        None => next,
                    };
                    for (cond, body) in arms.iter().rev() {
                        if cond.ty != Type::Bool {
                            return Err(denied("non-boolean branch"));
                        }
                        let yes = self.block(body, next, depth + 1, loop_targets)?;
                        other = self.point(PointKind::Branch, Some(cond), vec![yes, other])?;
                    }
                    other
                }
                TStmt::Loop { cond, body, update } => {
                    if cond.as_ref().is_some_and(|c| c.ty != Type::Bool) {
                        return Err(denied("non-boolean loop condition"));
                    }
                    // Reserve the header before descending, so backedges use
                    // an existing stable point ID. Continue reaches update,
                    // then the header; break reaches the loop continuation.
                    let head = self.point(PointKind::LoopHead, cond.as_ref(), vec![])?;
                    // The admitted update profile contains no direct loop
                    // transfers; nested loops still get their own targets.
                    let update_entry = self.block(update, head, depth + 1, None)?;
                    let body_entry = self.block(body, update_entry, depth + 1,
                        Some((next, update_entry)))?;
                    self.points[head].successors = if cond.is_some() {
                        vec![body_entry, next]
                    } else {
                        vec![body_entry]
                    };
                    head
                }
                TStmt::Break => {
                    let (target, _) = loop_targets.ok_or_else(|| denied("unbound break"))?;
                    self.point(PointKind::Break, None, vec![target])?
                }
                TStmt::Continue => {
                    let (_, target) = loop_targets.ok_or_else(|| denied("unbound continue"))?;
                    self.point(PointKind::Continue, None, vec![target])?
                }
                _ => return Err(denied("unmodelled control flow, memory or cleanup")),
            };
        }
        Ok(next)
    }
}

/// Derive a deterministic structural inventory from the checked typed program.
/// Point indices are reverse-construction IDs, local to each function. The
/// complete syntactic inventory includes unreachable points; only the entry
/// and successor edges describe reachability. No external facts are accepted.
/// This does not establish source loan completeness or emitter equivalence.
pub fn from_checked_program(program: &TProgram) -> Result<Vec<FunctionCensus>, Diagnostic> {
    if program.funcs.len() > MAX_FUNCTIONS || program.heap || program.strings
        || !program.actors.is_empty() || !program.generators.is_empty()
        || !program.globals.is_empty() || !program.imports.is_empty()
        || program.guest_eh || program.raise_tag || program.catch_depth
    {
        return Err(denied("program outside scalar census profile"));
    }
    let mut points = MAX_POINTS;
    let mut visits = MAX_EXPR_VISITS;
    let mut result = Vec::with_capacity(program.funcs.len());
    for function in &program.funcs {
        if function.params.iter().chain(&function.locals).any(|ty| !scalar(*ty))
            || !(function.ret == Type::Void || scalar(function.ret))
        {
            return Err(denied("non-scalar function signature or local"));
        }
        let mut builder = Builder {
            function, points: Vec::new(),
            remaining_points: &mut points, remaining_visits: &mut visits,
        };
        let exit = builder.point(PointKind::Exit, None, vec![])?;
        let entry = builder.block(&function.body, exit, 0, None)?;
        result.push(FunctionCensus {
            name: function.name.clone(), entry, points: builder.points,
        });
    }
    Ok(result)
}

const MAX_LIVE_SLOTS: usize = 4096;
const MAX_LIVE_WORD_WORK: usize = 8_000_000;

/// Diagnostic normal-flow may-liveness of scalar local slots, not loan
/// liveness. A slot is not an allocation or an authenticated loan identity.
/// Unreachable points are distinct from reachable points with no live slots.
#[derive(Debug)]
pub struct ScalarLiveness {
    pub function: String,
    slots: Vec<u32>,
    reachable: Vec<bool>,
    live_in: Vec<Vec<u64>>,
    live_out: Vec<Vec<u64>>,
}

impl ScalarLiveness {
    pub fn is_reachable(&self, point: usize) -> bool {
        self.reachable.get(point).copied().unwrap_or(false)
    }

    pub fn live_before(&self, point: usize, slot: u32) -> Option<bool> {
        self.contains(&self.live_in, point, slot)
    }

    pub fn live_after(&self, point: usize, slot: u32) -> Option<bool> {
        self.contains(&self.live_out, point, slot)
    }

    fn contains(&self, sets: &[Vec<u64>], point: usize, slot: u32) -> Option<bool> {
        if !self.is_reachable(point) {
            return None;
        }
        Some(self.slots.binary_search(&slot).is_ok_and(|i|
            sets[point][i / 64] & (1u64 << (i % 64)) != 0))
    }
}

fn solve(graph: &FunctionCensus, fuel: &mut usize) -> Result<ScalarLiveness, Diagnostic> {
    let n = graph.points.len();
    if n == 0 || n > MAX_POINTS || graph.entry >= n {
        return Err(denied("invalid liveness graph"));
    }
    let mut reachable = vec![false; n];
    let mut predecessors = vec![Vec::new(); n];
    let mut slots = BTreeSet::new();
    for (id, point) in graph.points.iter().enumerate() {
        if point.successors.len() > 2 || point.reads.len() > MAX_LIVE_SLOTS {
            return Err(denied("liveness point budget exhausted"));
        }
        for &successor in &point.successors {
            let pred = predecessors.get_mut(successor)
                .ok_or_else(|| denied("invalid liveness edge"))?;
            pred.push(id);
        }
        slots.extend(point.reads.iter().copied());
        if let PointKind::Set(slot) = point.kind {
            slots.insert(slot);
        }
        if slots.len() > MAX_LIVE_SLOTS {
            return Err(denied("liveness slot budget exhausted"));
        }
    }
    let slots: Vec<_> = slots.into_iter().collect();
    let words = slots.len().div_ceil(64);
    let mut visit = vec![graph.entry];
    reachable[graph.entry] = true;
    while let Some(id) = visit.pop() {
        for &succ in &graph.points[id].successors {
            if !reachable[succ] {
                reachable[succ] = true;
                visit.push(succ);
            }
        }
    }
    let mut live_in = vec![vec![0u64; words]; n];
    let mut live_out = live_in.clone();
    let mut queued = reachable.clone();
    // Reverse construction IDs usually process exits before predecessors.
    let mut work: VecDeque<_> = (0..n).filter(|&i| reachable[i]).collect();
    let mut out = vec![0u64; words];
    let mut input = out.clone();
    while let Some(id) = work.pop_front() {
        queued[id] = false;
        let point = &graph.points[id];
        // Bound the repeated word unions, copies, comparisons and sparse
        // read insertion. Count predecessor visits even for empty bitsets.
        let cost = 1 + words * (6 + point.successors.len())
            + point.reads.len() + predecessors[id].len();
        *fuel = fuel.checked_sub(cost)
            .ok_or_else(|| denied("liveness work budget exhausted"))?;
        out.fill(0);
        for &succ in &point.successors {
            for (dst, src) in out.iter_mut().zip(&live_in[succ]) {
                *dst |= *src;
            }
        }
        input.copy_from_slice(&out);
        if let PointKind::Set(slot) = point.kind {
            let i = slots.binary_search(&slot).expect("collected definition slot");
            input[i / 64] &= !(1u64 << (i % 64));
        }
        for slot in &point.reads {
            let i = slots.binary_search(slot).expect("collected read slot");
            input[i / 64] |= 1u64 << (i % 64);
        }
        live_out[id].copy_from_slice(&out);
        if live_in[id] != input {
            live_in[id].copy_from_slice(&input);
            for &pred in &predecessors[id] {
                if reachable[pred] && !queued[pred] {
                    queued[pred] = true;
                    work.push_back(pred);
                }
            }
        }
    }
    Ok(ScalarLiveness {
        function: graph.name.clone(), slots, reachable, live_in, live_out,
    })
}

/// Derive normal-flow scalar liveness directly from the compiler's checked
/// IR. A shared work budget bounds the entire program, and exhaustion returns
/// no partial result. No exceptional edges, loan issuance, or codegen proof.
pub fn scalar_liveness(program: &TProgram) -> Result<Vec<ScalarLiveness>, Diagnostic> {
    let graphs = from_checked_program(program)?;
    let mut fuel = MAX_LIVE_WORD_WORK;
    graphs.iter().map(|graph| solve(graph, &mut fuel)).collect()
}

#[cfg(test)]
mod liveness_budget_tests {
    use super::*;

    #[test]
    fn work_exhaustion_and_invalid_edges_return_no_partial_solution() {
        let mut graph = FunctionCensus {
            name: "test".into(), entry: 0,
            points: vec![Point {
                kind: PointKind::LoopHead, reads: vec![0], successors: vec![0],
            }],
        };
        assert_eq!(solve(&graph, &mut 0).unwrap_err().code, "E_TYPED_CENSUS");
        let solved = solve(&graph, &mut 100).unwrap();
        assert_eq!(solved.live_before(0, 0), Some(true));
        assert_eq!(solved.live_after(0, 0), Some(true));
        graph.points[0].successors[0] = 1;
        assert_eq!(solve(&graph, &mut 100).unwrap_err().code, "E_TYPED_CENSUS");
    }
}
