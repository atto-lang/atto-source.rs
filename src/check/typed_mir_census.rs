//! Compiler-owned, restricted typed-MIR scalar use/definition census.
//!
//! Built from the *checked* TProgram, not caller-authored lifetime facts.
//! This is a structural, read-only diagnostic, NOT a general borrow checker,
//! proof of source-to-emitted-Wasm coverage, or permission to emit code rejected
//! by the existing checker. A checked borrow issue survives in TStmt, but
//! source alias uses are not yet represented as typed-MIR loan events;
//! therefore every borrow issue FAILS CLOSED in this v0 census.
use std::collections::{BTreeMap, BTreeSet};

use crate::ast::Type;
use crate::check::{TExpr, TFunc, TProgram, TStmt, TE};
use crate::diag::{Diagnostic, Pos};

pub const CONTRACT: &str = "atto-typed-scalar-census-v0";
const MAX_CENSUS_FUNCTIONS: usize = 256;
const MAX_CENSUS_POINTS: usize = 4_096;
const MAX_CENSUS_ORIGINS: usize = 512;
const MAX_CENSUS_USES: usize = 100_000;
const MAX_EXPR_VISITS: usize = 100_000;
const MAX_EXPR_DEPTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    /// A compiler-emitted write to an SSA-versioned scalar local.
    Define { local_slot: u32, new_origin: u32 },
    /// Pure expression with all local reads independently enumerated.
    Evaluate,
    /// Terminal function return; carries all scalar use origins.
    Return,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    /// One-based, stable within this compiled function.
    pub instruction_id: u32,
    pub operation: Operation,
    pub used_origins: Vec<u32>,
    /// Checked arithmetic, numeric negation or comparison might trap.
    /// Not an exception-edge proof; no borrowed resources are admitted here.
    pub may_trap: bool,
    /// Pure ternary reads conservatively include both arms, but its
    /// internal conditional control-flow is not silently erased.
    pub has_conditional: bool,
    pub successor: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub function_index: u32,
    pub name: String,
    /// Parameters have initial origins (1..=params.len()) at entry.
    pub initial_parameter_origins: Vec<(u32, u32)>,
    pub points: Vec<Point>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Census {
    pub contract: &'static str,
    pub functions: Vec<Function>,
}
fn unsupported(message: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_TYPED_MIR_CENSUS_UNSUPPORTED", message)
}
fn budget(message: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_TYPED_MIR_CENSUS_BUDGET", message)
}
fn incomplete(message: &'static str) -> Diagnostic {
    Diagnostic::new(Pos::default(), "E_TYPED_MIR_CENSUS_INCOMPLETE", message)
}
fn simple_type(ty: Type) -> bool {
    matches!(ty, Type::Int | Type::Bool | Type::Float | Type::Void)
}
struct Builder {
    origins: BTreeMap<u32, u32>,
    next_origin: u32,
    points: Vec<Point>,
    use_facts: usize,
    expr_visits: usize,
    may_trap: bool,
    has_conditional: bool,
    slots: u32,
}
impl Builder {
    fn charge_expr(&mut self, depth: usize) -> Result<(), Diagnostic> {
        self.expr_visits = self.expr_visits.checked_add(1).ok_or_else(|| budget("expression work overflow"))?;
        if self.expr_visits > MAX_EXPR_VISITS || depth > MAX_EXPR_DEPTH {
            return Err(budget("typed expression depth/work exceeds bounded census"));
        }
        Ok(())
    }
    /// Enumerate all typed scalar reads; no pre-typed source analysis or
    /// externally supplied origin table can shrink this list.
    fn reads(&mut self, e: &TE, depth: usize, uses: &mut BTreeSet<u32>) -> Result<(), Diagnostic> {
        self.charge_expr(depth)?;
        if !simple_type(e.ty) {
            return Err(unsupported("non-scalar expression requires independent memory/loan proof"));
        }
        match &e.e {
            TExpr::I64(_) | TExpr::F64(_) | TExpr::Bool(_) => {}
            TExpr::Local(slot) => {
                if *slot >= self.slots {
                    return Err(incomplete("typed expression uses out-of-range local slot"));
                }
                let origin = self
                    .origins
                    .get(slot)
                    .ok_or_else(|| incomplete("typed expression reads local before definition"))?;
                uses.insert(*origin);
            }
            TExpr::Neg(inner) | TExpr::FNeg(inner) => {
                self.may_trap = true;
                self.reads(inner, depth + 1, uses)?;
            }
            TExpr::Not(inner) | TExpr::IntToFloat(inner) => self.reads(inner, depth + 1, uses)?,
            TExpr::Bin(_, a, b) => {
                // Integer operations are checked and may trap. Conservatively
                // mark EVERY typed binary rather than guessing a safe opcode.
                self.may_trap = true;
                self.reads(a, depth + 1, uses)?;
                self.reads(b, depth + 1, uses)?;
            }
            TExpr::Builtin(_, a, b) => {
                self.reads(a, depth + 1, uses)?;
                self.reads(b, depth + 1, uses)?;
            }
            TExpr::Ternary(c, a, b) => {
                self.has_conditional = true;
                // Both arms contribute MAY reads, with a distinct conditional
                // flag rather than a fabricated single-path effect proof.
                self.reads(c, depth + 1, uses)?;
                self.reads(a, depth + 1, uses)?;
                self.reads(b, depth + 1, uses)?;
            }
            // Effectful calls, printing, allocation, exceptions, member/heap
            // access, captures, actor messaging and tagged results cannot
            // silently be counted as pure scalar reads.
            _ => return Err(unsupported("typed expression has unmodeled effect, alias, or control path")),
        }
        if self.use_facts.saturating_add(uses.len()) > MAX_CENSUS_USES {
            return Err(budget("typed origin use inventory exceeded"));
        }
        Ok(())
    }
    fn add_point(&mut self, operation: Operation, uses: BTreeSet<u32>) -> Result<(), Diagnostic> {
        if self.points.len() >= MAX_CENSUS_POINTS {
            return Err(budget("typed MIR exceeds point limit"));
        }
        self.use_facts = self
            .use_facts
            .checked_add(uses.len())
            .ok_or_else(|| budget("typed use inventory overflow"))?;
        if self.use_facts > MAX_CENSUS_USES {
            return Err(budget("typed origin use inventory exceeded"));
        }
        let id = self.points.len() as u32 + 1;
        if let Some(previous) = self.points.last_mut() {
            previous.successor = Some(id);
        }
        self.points.push(Point {
            instruction_id: id,
            operation,
            used_origins: uses.into_iter().collect(),
            may_trap: self.may_trap,
            has_conditional: self.has_conditional,
            successor: None,
        });
        self.may_trap = false;
        self.has_conditional = false;
        Ok(())
    }
    fn walk_stmt(&mut self, stmt: &TStmt) -> Result<(), Diagnostic> {
        match stmt {
            TStmt::Set(slot, value) => {
                if *slot >= self.slots {
                    return Err(incomplete("typed set targets out-of-range local slot"));
                }
                let mut uses = BTreeSet::new();
                self.reads(value, 0, &mut uses)?;
                if self.next_origin as usize > MAX_CENSUS_ORIGINS {
                    return Err(budget("SSA origin count exceeded"));
                }
                let new_origin = self.next_origin;
                self.next_origin += 1;
                self.origins.insert(*slot, new_origin);
                self.add_point(
                    Operation::Define {
                        local_slot: *slot,
                        new_origin,
                    },
                    uses,
                )?;
            }
            TStmt::Expr(value) => {
                let mut uses = BTreeSet::new();
                self.reads(value, 0, &mut uses)?;
                self.add_point(Operation::Evaluate, uses)?;
            }
            TStmt::Return(value) => {
                let mut uses = BTreeSet::new();
                if let Some(value) = value {
                    self.reads(value, 0, &mut uses)?;
                }
                self.add_point(Operation::Return, uses)?;
            }
            TStmt::Block(stmts) if !stmts.is_empty() => {
                for child in stmts {
                    if self.points.last().is_some_and(|p| matches!(p.operation, Operation::Return)) {
                        return Err(incomplete("typed block contains operations after terminal return"));
                    }
                    self.walk_stmt(child)?;
                }
            }
            TStmt::ScalarBorrowIssue { .. } => {
                // Source-checked loan event is retained in typed IR but
                // alias reads/ends are not yet represented. Never create a
                // "complete" loan-use inventory by guessing their absence.
                return Err(unsupported("scalar borrow alias requires typed MIR loan-use/end census"));
            }
            _ => return Err(unsupported("nonlinear CFG, empty opaque block or unmodeled ownership effect")),
        }
        Ok(())
    }
}

fn function(index: usize, func: &TFunc) -> Result<Function, Diagnostic> {
    if !simple_type(func.ret)
        || func
            .params
            .iter()
            .chain(func.locals.iter())
            .any(|&ty| !simple_type(ty) || ty == Type::Void)
    {
        return Err(unsupported("aggregate, handle and reference storage need typed ownership proof"));
    }
    let slots = func
        .params
        .len()
        .checked_add(func.locals.len())
        .ok_or_else(|| budget("local slot count overflow"))?;
    if slots > MAX_CENSUS_ORIGINS {
        return Err(budget("too many scalar slots"));
    }
    let mut builder = Builder {
        origins: BTreeMap::new(),
        next_origin: 1,
        points: Vec::new(),
        use_facts: 0,
        expr_visits: 0,
        may_trap: false,
        has_conditional: false,
        slots: slots as u32,
    };
    let mut initial_parameter_origins = Vec::new();
    for slot in 0..func.params.len() as u32 {
        initial_parameter_origins.push((slot, builder.next_origin));
        builder.origins.insert(slot, builder.next_origin);
        builder.next_origin += 1;
    }
    for stmt in &func.body {
        if builder.points.last().is_some_and(|p| matches!(p.operation, Operation::Return)) {
            return Err(incomplete("typed MIR contains operations after terminal return"));
        }
        builder.walk_stmt(stmt)?;
    }
    if !builder.points.last().is_some_and(|p| matches!(p.operation, Operation::Return)) {
        return Err(unsupported("implicit return requires an explicit terminal MIR event"));
    }
    Ok(Function {
        function_index: index as u32,
        name: func.name.clone(),
        initial_parameter_origins,
        points: builder.points,
    })
}

/// Require a closed, straight-line typed scalar profile and produce an
/// actual compiler-owned read/SSA definition/return inventory.
/// This does not authenticate source/emitted-code completeness, CFG beyond
/// straight-line instructions, or any borrow loan-use inventory.
pub fn from_checked_program(program: &TProgram) -> Result<Census, Diagnostic> {
    if program.funcs.is_empty()
        || program.funcs.len() > MAX_CENSUS_FUNCTIONS
        || !program.generators.is_empty()
        || !program.actors.is_empty()
        || program.heap
        || program.guest_eh
    {
        return Err(unsupported(
            "program has unsupported generators, actors, memory or exception effects",
        ));
    }
    let mut functions = Vec::with_capacity(program.funcs.len());
    for (i, func) in program.funcs.iter().enumerate() {
        functions.push(function(i, func)?);
    }
    Ok(Census {
        contract: CONTRACT,
        functions,
    })
}

/// One borrow issuance whose owner identity and exclusivity were validated by
/// the *actual* AttoLang type/ownership pass. Does NOT include loan uses,
/// kills, escaped references or emitted-code attestation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedBorrowIssue {
    pub function_index: u32,
    pub instruction_ordinal: u32,
    pub loan_id: u32,
    pub owner_slot: u32,
    pub alias: String,
    pub exclusive: bool,
}

/// Observe source-checked scalar borrow ISSUANCES on a restricted straight-line
/// typed instruction stream. This partial census must never be mistaken for a
/// complete loan-use/liveness certificate.
pub fn checked_borrow_issues(program: &TProgram) -> Result<Vec<CheckedBorrowIssue>, Diagnostic> {
    if program.funcs.len() > MAX_CENSUS_FUNCTIONS {
        return Err(budget("too many functions in borrow issue census"));
    }
    let mut issues = Vec::new();
    for (index, f) in program.funcs.iter().enumerate() {
        if f.body.len() > MAX_CENSUS_POINTS {
            return Err(budget("too many typed statements in borrow issue census"));
        }
        for (ordinal, statement) in f.body.iter().enumerate() {
            if let TStmt::ScalarBorrowIssue {
                loan_id,
                owner_slot,
                alias,
                exclusive,
            } = statement
            {
                if *loan_id == 0 || alias.is_empty() || *owner_slot as usize >= f.params.len() + f.locals.len() {
                    return Err(incomplete("checked scalar borrow has malformed owner/alias provenance"));
                }
                issues.push(CheckedBorrowIssue {
                    function_index: index as u32,
                    instruction_ordinal: ordinal as u32 + 1,
                    loan_id: *loan_id,
                    owner_slot: *owner_slot,
                    alias: alias.clone(),
                    exclusive: *exclusive,
                });
            } else if matches!(
                statement,
                TStmt::If(..) | TStmt::Loop { .. } | TStmt::ForIn { .. } | TStmt::Block(..)
            ) {
                return Err(unsupported("nonlinear or nested borrow issue sites require CFG census"));
            }
        }
    }
    Ok(issues)
}
