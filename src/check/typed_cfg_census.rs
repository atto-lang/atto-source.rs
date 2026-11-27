//! Independent, bounded read/definition census over the checked scalar IR.
//! This is diagnostic-only: the current typed IR erases loan issuance and
//! result provenance, so a successful census must never authorize borrowing.
use super::{TExpr, TFunc, TProgram, TStmt, TE};
use crate::ast::Type;
use crate::diag::{Diagnostic, Pos};
use std::collections::BTreeSet;

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
