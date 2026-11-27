//! Task state-machine planning for `susp` callables (atto.task.v1).
//!
//! The checker types a suspending body with the ordinary statement checker;
//! `await` appears as `TStmt::Suspend`. This planner flattens only the
//! statements that *must* be resumable — those containing a suspension, a
//! `return`, or a `break`/`continue` that leaves them — into a PC-addressed
//! step list. Every other statement stays an opaque `Run` segment emitted by
//! the ordinary emitter, so suspending callables keep the full admitted
//! language between suspension points.
//!
//! Every local (parameters included) lives in a fixed-size frame slot and is
//! reloaded on resume, so no value depends on a Wasm stack that a suspension
//! discarded. Constructs whose unwinding semantics need continuation-aware
//! cleanup (try/defer/recover, iterator loops, critical state scopes) are
//! rejected when a suspension or return crosses them.
use super::*;

/// Frame slots per task (parameters + locals); fixed so frames recycle.
pub const MAX_TASK_SLOTS: usize = 64;
pub(super) const MAX_TASK_STEPS: usize = 2048;

struct LoopCx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct Planner<'a> {
    f: &'a TFunc,
    steps: Vec<TTaskStep>,
    open_run: bool,
    loops: Vec<LoopCx>,
    pos: Pos,
}

pub(super) fn plan(f: &TFunc, function: u32, pos: Pos) -> R<TTask> {
    if f.params.len() + f.locals.len() > MAX_TASK_SLOTS {
        return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", format!(
            "susp callable '{}' needs more than {MAX_TASK_SLOTS} frame slots", f.name)));
    }
    if stmt_exists(&f.body, &|s| matches!(s, TStmt::UnwindFrame { .. })) {
        return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
            "try/defer/recover in a susp callable needs continuation-aware unwind lowering"));
    }
    let mut p = Planner { f, steps: Vec::new(), open_run: false, loops: Vec::new(), pos };
    p.block(&f.body)?;
    p.push(if f.ret == Type::Void { TTaskStep::Return(None) } else { TTaskStep::Unreachable });
    if p.steps.len() > MAX_TASK_STEPS {
        return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", format!(
            "susp callable '{}' exceeds {MAX_TASK_STEPS} resumable steps", f.name)));
    }
    Ok(TTask { function, steps: p.steps })
}

impl Planner<'_> {
    fn push(&mut self, step: TTaskStep) -> usize {
        self.open_run = false;
        self.steps.push(step);
        self.steps.len() - 1
    }

    /// Current PC as a jump target: later code must not merge into a
    /// segment that started before the label.
    fn label(&mut self) -> u32 {
        self.open_run = false;
        self.steps.len() as u32
    }

    fn run(&mut self, s: &TStmt) {
        if self.open_run {
            if let Some(TTaskStep::Run(body)) = self.steps.last_mut() {
                body.push(s.clone());
                return;
            }
        }
        self.steps.push(TTaskStep::Run(vec![s.clone()]));
        self.open_run = true;
    }

    fn patch(&mut self, at: usize, target: u32) {
        match &mut self.steps[at] {
            TTaskStep::Guard { on_false, .. } => *on_false = target,
            TTaskStep::Jump { to, .. } => *to = target,
            _ => unreachable!("only guards and jumps are patched"),
        }
    }

    fn slot_type(&self, slot: u32) -> Type {
        let n = self.f.params.len() as u32;
        if slot < n { self.f.params[slot as usize] } else { self.f.locals[(slot - n) as usize] }
    }

    fn block(&mut self, body: &[TStmt]) -> R<()> {
        for s in body {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &TStmt) -> R<()> {
        if !must_flatten(s) {
            self.run(s);
            return Ok(());
        }
        match s {
            TStmt::Suspend { op, dest } => {
                self.push(TTaskStep::Suspend(op.clone()));
                if let Some(dest) = *dest {
                    let ty = self.slot_type(dest);
                    self.push(TTaskStep::Receive { dest, ty });
                }
            }
            TStmt::Return(v) => { self.push(TTaskStep::Return(v.clone())); }
            TStmt::Block(b) => self.block(b)?,
            TStmt::If(arms, otherwise) => {
                let mut ends = Vec::new();
                for (condition, body) in arms {
                    let guard = self.push(TTaskStep::Guard { condition: condition.clone(), on_false: 0 });
                    self.block(body)?;
                    ends.push(self.push(TTaskStep::Jump { to: 0, back: false }));
                    let next = self.label();
                    self.patch(guard, next);
                }
                if let Some(body) = otherwise {
                    self.block(body)?;
                }
                let end = self.label();
                for j in ends { self.patch(j, end); }
            }
            TStmt::Loop { cond, body, update } => {
                let top = self.label();
                let guard = cond.as_ref().map(|c|
                    self.push(TTaskStep::Guard { condition: c.clone(), on_false: 0 }));
                self.loops.push(LoopCx { breaks: Vec::new(), continues: Vec::new() });
                self.block(body)?;
                let cont = self.label();
                self.block(update)?;
                self.push(TTaskStep::Jump { to: top, back: true });
                let end = self.label();
                let cx = self.loops.pop().expect("loop context");
                if let Some(g) = guard { self.patch(g, end); }
                for b in cx.breaks { self.patch(b, end); }
                for c in cx.continues { self.patch(c, cont); }
            }
            TStmt::Break | TStmt::Continue => {
                let at = self.push(TTaskStep::Jump { to: 0, back: matches!(s, TStmt::Continue) });
                let cx = self.loops.last_mut().ok_or_else(|| Diagnostic::new(self.pos, "E_UNSUPPORTED",
                    "break/continue crosses a construct that cannot be resumed"))?;
                if matches!(s, TStmt::Break) { cx.breaks.push(at) } else { cx.continues.push(at) }
            }
            TStmt::ForIn { .. } => return Err(Diagnostic::new(self.pos, "E_UNSUPPORTED",
                "await/return inside a generator for-in loop of a susp callable needs owned-iterator suspension")),
            TStmt::StateCriticalWrite(_) => return Err(Diagnostic::new(self.pos, "E_BORROW",
                "a critical state loan cannot cross await")),
            _ => return Err(Diagnostic::new(self.pos, "E_UNSUPPORTED",
                "await/return inside try/defer/recover of a susp callable is not lowered yet")),
        }
        Ok(())
    }
}

/// A statement must be split into resumable steps when it suspends, returns,
/// or branches out of itself.
fn must_flatten(s: &TStmt) -> bool {
    stmt_exists(std::slice::from_ref(s), &|t| matches!(t, TStmt::Suspend { .. } | TStmt::Return(_)))
        || escapes(s, 0)
}

fn escapes(s: &TStmt, depth: u32) -> bool {
    match s {
        TStmt::Break | TStmt::Continue => depth == 0,
        TStmt::Loop { body, update, .. } =>
            body.iter().chain(update).any(|t| escapes(t, depth + 1)),
        TStmt::ForIn { body, .. } => body.iter().any(|t| escapes(t, depth + 1)),
        TStmt::If(arms, otherwise) =>
            arms.iter().any(|(_, b)| b.iter().any(|t| escapes(t, depth)))
                || otherwise.as_ref().is_some_and(|b| b.iter().any(|t| escapes(t, depth))),
        TStmt::Block(b) | TStmt::StateCriticalWrite(b) => b.iter().any(|t| escapes(t, depth)),
        TStmt::Try { body, catch, finally, .. } =>
            body.iter().chain(catch.iter().flatten()).chain(finally.iter().flatten())
                .any(|t| escapes(t, depth)),
        _ => false,
    }
}
