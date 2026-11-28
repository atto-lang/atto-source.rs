//! AST desugaring for suspending callables (atto.task.v1).
//!
//! 1. Every `task { body }` / `do task { body }` inside a `susp`/`suspends`
//!    callable becomes a synthetic private suspending callable whose
//!    parameters are the body's by-value captures. The expression becomes
//!    `SpawnTask { func, args }`. Captures must be immutable bindings (`val`,
//!    `const`, non-`mut` parameters): a task never aliases a caller's
//!    mutable local, so no borrow can cross the task boundary.
//! 2. Nested `await` operands are hoisted into fresh `val` temporaries in
//!    left-to-right evaluation order, so the checker only ever sees `await`
//!    as a statement-level operand (`val x = await e`, `x = await e`,
//!    `await e;`, `return await e`). Awaits behind short-circuit operators or
//!    ternary arms are rejected rather than evaluated eagerly.
//!    `while (await c) do .. done` becomes `loop { val t = await c; if !t
//!    break; .. }`, re-evaluating the operand before every iteration.
//!
//! Nothing here emits code; the checker types the result and the task
//! planner proves which statements can cross a suspension point.
use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use std::collections::{BTreeMap, BTreeSet};
type R<T> = Result<T, Diagnostic>;

const MAX_DEPTH: usize = 128;
pub(super) const MAX_TASK_BODIES: usize = 64;

fn err(pos: Pos, code: &'static str, m: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, m)
}

pub(super) fn desugar(p: &Program) -> R<Program> {
    let any = p.funcs.iter().any(|f| f.is_suspending || mentions_suspension(&f.body));
    if !any {
        return Ok(p.clone());
    }
    let mut out = p.clone();
    // Ret types of declared callables, for capture type inference.
    let mut rets: BTreeMap<(String, usize), Type> = BTreeMap::new();
    for f in &p.funcs {
        rets.insert((f.name.clone(), f.params.len()), f.ret);
    }
    let mut cx = Cx { rets, generated: Vec::new(), temps: 0 };
    let mut i = 0;
    while i < out.funcs.len() {
        if out.funcs[i].is_suspending {
            let mut f = out.funcs[i].clone();
            cx.function(&mut f)?;
            out.funcs[i] = f;
            // Synthetic task callables are processed by this same loop.
            for g in cx.generated.drain(..) {
                if out.funcs.len() - p.funcs.len() >= MAX_TASK_BODIES {
                    return Err(err(g.pos, "E_SOURCE_LIMIT",
                        format!("more than {MAX_TASK_BODIES} task bodies")));
                }
                cx.rets.insert((g.name.clone(), g.params.len()), g.ret);
                out.funcs.push(g);
            }
        } else if let Some(pos) = first_suspension(&out.funcs[i].body) {
            return Err(err(pos, "E_SUSPEND", format!(
                "'{}' must be declared susp/suspends to await or start a task", out.funcs[i].name)));
        }
        i += 1;
    }
    Ok(out)
}

struct Cx {
    rets: BTreeMap<(String, usize), Type>,
    generated: Vec<Func>,
    temps: u32,
}

#[derive(Clone, Copy)]
struct Binding { ty: Option<Type>, mutable: bool }

impl Cx {
    fn function(&mut self, f: &mut Func) -> R<()> {
        // Environment of every binding visible somewhere in the function. A
        // name bound twice with different types or mutability is ambiguous
        // and can only be captured after the user renames one binding.
        let mut env: BTreeMap<String, Option<Binding>> = BTreeMap::new();
        for q in &f.params {
            env.insert(q.name.clone(), Some(Binding { ty: Some(q.ty), mutable: q.mutable }));
        }
        collect_bindings(&f.body, &mut env, &self.rets, 0)?;
        let mut counter = 0u32;
        let mut body = std::mem::take(&mut f.body);
        self.extract_susp_lambdas(&mut body, &f.name, &env)?;
        let body = self.extract_stmts(body, &f.name, &env, &mut counter, 0)?;
        f.body = self.hoist_block(body, 0)?;
        Ok(())
    }

    // ---------- task extraction ----------

    fn extract_stmts(&mut self, body: Vec<Stmt>, parent: &str,
        env: &BTreeMap<String, Option<Binding>>, n: &mut u32, depth: usize) -> R<Vec<Stmt>> {
        body.into_iter().map(|s| self.extract_stmt(s, parent, env, n, depth + 1)).collect()
    }

    fn extract_stmt(&mut self, s: Stmt, parent: &str,
        env: &BTreeMap<String, Option<Binding>>, n: &mut u32, depth: usize) -> R<Stmt> {
        if depth > MAX_DEPTH {
            return Err(err(stmt_pos(&s), "E_NESTING", "task desugaring nesting limit exceeded"));
        }
        let mut s = s;
        let mut fail = None;
        map_stmt_exprs(&mut s, &mut |e| {
            if fail.is_none() {
                if let Err(d) = self.extract_expr(e, parent, env, n, depth + 1) { fail = Some(d); }
            }
        });
        if let Some(d) = fail { return Err(d); }
        Ok(match s {
            Stmt::If { arms, otherwise, pos } => Stmt::If {
                arms: arms.into_iter().map(|(c, b)| Ok((c, self.extract_stmts(b, parent, env, n, depth)?)))
                    .collect::<R<_>>()?,
                otherwise: otherwise.map(|b| self.extract_stmts(b, parent, env, n, depth)).transpose()?,
                pos,
            },
            Stmt::While { cond, body, pos } =>
                Stmt::While { cond, body: self.extract_stmts(body, parent, env, n, depth)?, pos },
            Stmt::Loop { body, pos } =>
                Stmt::Loop { body: self.extract_stmts(body, parent, env, n, depth)?, pos },
            Stmt::For { init, cond, update, body, pos } =>
                Stmt::For { init, cond, update, body: self.extract_stmts(body, parent, env, n, depth)?, pos },
            Stmt::ForIn { name, source, body, pos } =>
                Stmt::ForIn { name, source, body: self.extract_stmts(body, parent, env, n, depth)?, pos },
            Stmt::Block(b) => Stmt::Block(self.extract_stmts(b, parent, env, n, depth)?),
            Stmt::Match { subject, arms, pos } => Stmt::Match {
                subject,
                arms: arms.into_iter().map(|mut a| {
                    a.body = self.extract_stmts(std::mem::take(&mut a.body), parent, env, n, depth)?;
                    Ok(a)
                }).collect::<R<_>>()?,
                pos,
            },
            other => other,
        })
    }

    fn extract_expr(&mut self, e: &mut Expr, parent: &str,
        env: &BTreeMap<String, Option<Binding>>, n: &mut u32, depth: usize) -> R<()> {
        if depth > MAX_DEPTH {
            return Err(err(e.pos, "E_NESTING", "task desugaring nesting limit exceeded"));
        }
        let mut fail = None;
        map_child_exprs(e, &mut |c| {
            if fail.is_none() {
                if let Err(d) = self.extract_expr(c, parent, env, n, depth + 1) { fail = Some(d); }
            }
        });
        if let Some(d) = fail { return Err(d); }
        let ExprKind::Task { body, detached } = &mut e.kind else { return Ok(()) };
        let body = std::mem::take(body);
        let detached = *detached;
        // Names used inside the body that it does not bind itself.
        let mut used = BTreeSet::new();
        let mut assigned = BTreeSet::new();
        let mut bound = BTreeSet::new();
        uses_in_stmts(&body, &mut used, &mut assigned, &mut bound);
        let mut params = Vec::new();
        let mut args = Vec::new();
        for name in used.iter().filter(|u| !bound.contains(*u)) {
            let Some(binding) = env.get(name) else { continue };
            let Some(Binding { ty, mutable }) = *binding else {
                return Err(err(e.pos, "E_TASK_CAPTURE", format!(
                    "task captures '{name}', which is bound more than once with different types; rename one binding")));
            };
            if mutable {
                return Err(err(e.pos, "E_TASK_CAPTURE", format!(
                    "task captures mutable binding '{name}' by value; copy it into a val first")));
            }
            let Some(ty) = ty else {
                return Err(err(e.pos, "E_TASK_CAPTURE", format!(
                    "cannot infer the type of captured '{name}'; annotate its binding")));
            };
            params.push(Param { name: name.clone(), ty, mutable: false, pos: e.pos });
            args.push(Expr { kind: ExprKind::Var(name.clone()), pos: e.pos });
        }
        if let Some(name) = assigned.iter().find(|a| !bound.contains(*a) && env.contains_key(*a)) {
            return Err(err(e.pos, "E_TASK_CAPTURE", format!(
                "task body cannot assign captured '{name}'; tasks own by-value copies")));
        }
        let returns_value = returns_value(&body);
        if detached && returns_value {
            return Err(err(e.pos, "E_TYPE", "do task { } is detached and cannot return a value"));
        }
        if !detached && !returns_value {
            return Err(err(e.pos, "E_TYPE", "task { } must return an int (it produces a Future<int>)"));
        }
        *n += 1;
        let name = format!("{parent}__task{n}");
        self.generated.push(Func {
            name: name.clone(),
            pos: e.pos,
            is_generator: false,
            is_suspending: true,
            is_pure: false,
            declared_effects: None,
            params,
            ret: if detached { Type::Void } else { Type::Int },
            body,
        });
        e.kind = ExprKind::SpawnTask { func: name, args, detached };
        Ok(())
    }

    // ---------- suspending lambdas ----------

    /// `val f = susp |[T] p, ..| [: R] -> body` becomes a synthetic susp
    /// callable `{parent}__susplambda{n}(p.., captures..)`; every direct call
    /// `f(args)` is rewritten to pass the by-value captures. Any other use of
    /// `f` would let the lambda escape and is rejected.
    fn extract_susp_lambdas(&mut self, body: &mut Vec<Stmt>,
        parent: &str, env: &BTreeMap<String, Option<Binding>>) -> R<()> {
        let mut found: Vec<SuspLambdaBind> = Vec::new();
        take_susp_lambda_binds(body, &mut found, 0)?;
        for (index, (name, pos, params, ret, lambda_body, mutable)) in found.into_iter().enumerate() {
            if mutable {
                return Err(err(pos, "E_UNSUPPORTED", "bind a suspending lambda with val; it cannot be reassigned"));
            }
            let mut calls = Vec::new();
            collect_calls(body, &name, &mut calls);
            let first = calls.first().ok_or_else(|| err(pos, "E_UNUSED",
                format!("suspending lambda '{name}' is never called")))?;
            let mut typed = Vec::with_capacity(params.len());
            for (i, (p, ty)) in params.iter().enumerate() {
                let ty = match ty {
                    Some(t) => *t,
                    None => first.get(i).and_then(|a| infer(a, env, &self.rets)).ok_or_else(|| err(pos, "E_INFER",
                        format!("annotate suspending lambda parameter '{p}' (its first call does not fix its type)")))?,
                };
                typed.push((p.clone(), ty));
            }
            if calls.iter().any(|args| args.len() != typed.len()) {
                return Err(err(pos, "E_ARITY", format!("suspending lambda '{name}' expects {} arguments", typed.len())));
            }
            let mut lambda_env = env.clone();
            for (p, t) in &typed {
                lambda_env.insert(p.clone(), Some(Binding { ty: Some(*t), mutable: false }));
            }
            let ret = match ret {
                Some(t) => t,
                None => match lambda_body.as_slice() {
                    [Stmt::Return(Some(e), _)] => infer(e, &lambda_env, &self.rets).unwrap_or(Type::Int),
                    _ => if returns_value(&lambda_body) { Type::Int } else { Type::Void },
                },
            };
            let (mut used, mut assigned, mut bound) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
            uses_in_stmts(&lambda_body, &mut used, &mut assigned, &mut bound);
            let mut captures = Vec::new();
            for c in used.iter().filter(|u| !bound.contains(*u) && !typed.iter().any(|(p, _)| p == *u)) {
                let Some(binding) = env.get(c) else { continue };
                let Some(Binding { ty: Some(ty), mutable: false }) = *binding else {
                    return Err(err(pos, "E_TASK_CAPTURE", format!(
                        "suspending lambda captures '{c}', which must be an immutable binding of known type")));
                };
                if assigned.contains(c) {
                    return Err(err(pos, "E_TASK_CAPTURE", format!("suspending lambda cannot assign captured '{c}'")));
                }
                captures.push((c.clone(), ty));
            }
            let synthetic = format!("{parent}__susplambda{}", index + 1);
            let mut fparams: Vec<Param> = typed.iter().map(|(n, t)| Param { name: n.clone(), ty: *t, mutable: false, pos }).collect();
            fparams.extend(captures.iter().map(|(n, t)| Param { name: n.clone(), ty: *t, mutable: false, pos }));
            self.generated.push(Func {
                name: synthetic.clone(), pos, is_generator: false, is_suspending: true, is_pure: false,
                declared_effects: None, params: fparams, ret, body: lambda_body,
            });
            self.rets.insert((synthetic.clone(), typed.len() + captures.len()), ret);
            let extra: Vec<Expr> = captures.iter().map(|(n, _)| Expr { kind: ExprKind::Var(n.clone()), pos }).collect();
            let mut escaped = None;
            rewrite_lambda_uses(body, &name, &synthetic, &extra, &mut escaped);
            if let Some(at) = escaped {
                return Err(err(at, "E_UNSUPPORTED", format!(
                    "suspending lambda '{name}' can only be called directly (await {name}(..)); it cannot escape as a value")));
            }
        }
        Ok(())
    }

    // ---------- await hoisting ----------

    fn temp(&mut self, pos: Pos) -> (String, Expr) {
        self.temps += 1;
        let name = format!("__await{}", self.temps);
        (name.clone(), Expr { kind: ExprKind::Var(name), pos })
    }

    fn hoist_block(&mut self, body: Vec<Stmt>, depth: usize) -> R<Vec<Stmt>> {
        let mut out = Vec::with_capacity(body.len());
        for s in body {
            self.hoist_stmt(s, &mut out, depth + 1)?;
        }
        Ok(out)
    }

    fn hoist_stmt(&mut self, s: Stmt, out: &mut Vec<Stmt>, depth: usize) -> R<()> {
        if depth > MAX_DEPTH {
            return Err(err(stmt_pos(&s), "E_NESTING", "await hoisting nesting limit exceeded"));
        }
        let stmt = match s {
            Stmt::Bind { kind, name, ty, init, pos } => {
                let init = self.top(init, out, true)?;
                Stmt::Bind { kind, name, ty, init, pos }
            }
            Stmt::Assign { name, value, pos } => {
                let value = self.top(value, out, true)?;
                Stmt::Assign { name, value, pos }
            }
            Stmt::Expr(e) => Stmt::Expr(self.top(e, out, true)?),
            Stmt::Return(Some(e), pos) => Stmt::Return(Some(self.top(e, out, true)?), pos),
            Stmt::Throw(e, pos) => Stmt::Throw(self.top(e, out, false)?, pos),
            Stmt::Raise(e, pos) => Stmt::Raise(self.top(e, out, false)?, pos),
            Stmt::FieldAssign { target, field, value, pos } => {
                let target = self.top(target, out, false)?;
                let value = self.top(value, out, false)?;
                Stmt::FieldAssign { target, field, value, pos }
            }
            Stmt::If { arms, otherwise, pos } => {
                let mut new_arms = Vec::with_capacity(arms.len());
                for (i, (cond, body)) in arms.into_iter().enumerate() {
                    let cond = if i == 0 {
                        self.top(cond, out, false)?
                    } else {
                        if contains_await(&cond) {
                            return Err(err(cond.pos, "E_UNSUPPORTED",
                                "await in an elif condition; bind it before the if"));
                        }
                        cond
                    };
                    new_arms.push((cond, self.hoist_block(body, depth)?));
                }
                let otherwise = otherwise.map(|b| self.hoist_block(b, depth)).transpose()?;
                Stmt::If { arms: new_arms, otherwise, pos }
            }
            Stmt::While { cond, body, pos } if contains_await(&cond) => {
                // loop { <hoisted>; if !(cond) { break; } body }
                let mut inner = Vec::new();
                let cond = self.top(cond, &mut inner, false)?;
                inner.push(Stmt::If {
                    arms: vec![(Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(cond)), pos },
                        vec![Stmt::Break(pos)])],
                    otherwise: None,
                    pos,
                });
                inner.extend(self.hoist_block(body, depth)?);
                Stmt::Loop { body: inner, pos }
            }
            Stmt::While { cond, body, pos } =>
                Stmt::While { cond, body: self.hoist_block(body, depth)?, pos },
            Stmt::Loop { body, pos } => Stmt::Loop { body: self.hoist_block(body, depth)?, pos },
            Stmt::For { init, cond, update, body, pos } => {
                if cond.as_ref().is_some_and(contains_await)
                    || init.as_deref().is_some_and(stmt_contains_await)
                    || update.as_deref().is_some_and(stmt_contains_await) {
                    return Err(err(pos, "E_UNSUPPORTED",
                        "await in a for header; use a while or loop with await in its body"));
                }
                Stmt::For { init, cond, update, body: self.hoist_block(body, depth)?, pos }
            }
            Stmt::ForIn { name, source, body, pos } => {
                let source = self.top(source, out, false)?;
                Stmt::ForIn { name, source, body: self.hoist_block(body, depth)?, pos }
            }
            Stmt::Block(b) => Stmt::Block(self.hoist_block(b, depth)?),
            Stmt::Match { subject, arms, pos } => {
                let subject = self.top(subject, out, false)?;
                let mut new_arms = Vec::with_capacity(arms.len());
                for mut a in arms {
                    if a.guard.as_ref().is_some_and(contains_await) {
                        return Err(err(a.pos, "E_UNSUPPORTED", "await in a match guard"));
                    }
                    a.body = self.hoist_block(std::mem::take(&mut a.body), depth)?;
                    new_arms.push(a);
                }
                Stmt::Match { subject, arms: new_arms, pos }
            }
            other => {
                if stmt_contains_await(&other) {
                    return Err(err(stmt_pos(&other), "E_UNSUPPORTED",
                        "await inside try/defer/recover/critical scopes needs continuation unwind lowering"));
                }
                other
            }
        };
        out.push(stmt);
        Ok(())
    }

    /// Rewrite `e` so that awaits and task starts only remain at its root
    /// (when `root_ok`), hoisting every other occurrence into `out`.
    fn top(&mut self, e: Expr, out: &mut Vec<Stmt>, root_ok: bool) -> R<Expr> {
        self.hoist_expr(e, out, root_ok, 0)
    }

    fn hoist_expr(&mut self, mut e: Expr, out: &mut Vec<Stmt>, root_ok: bool, depth: usize) -> R<Expr> {
        if depth > MAX_DEPTH {
            return Err(err(e.pos, "E_NESTING", "await hoisting nesting limit exceeded"));
        }
        match &e.kind {
            ExprKind::Binary(BinOp::And | BinOp::Or, _, b) if contains_await(b) =>
                return Err(err(e.pos, "E_UNSUPPORTED",
                    "await on the right of a short-circuit operator; bind it first")),
            ExprKind::Ternary(_, a, b) if contains_await(a) || contains_await(b) =>
                return Err(err(e.pos, "E_UNSUPPORTED", "await inside a ternary arm or behind a short-circuit operator; bind it first or use if/else")),
            ExprKind::Lambda { .. } if contains_await(&e) =>
                return Err(err(e.pos, "E_UNSUPPORTED",
                    "suspending lambdas are not lowered yet; declare a susp fnc")),
            _ => {}
        }
        // Children first, in evaluation order.
        let mut fail = None;
        let mut hoisted = Vec::new();
        map_child_exprs(&mut e, &mut |c| {
            if fail.is_some() { return; }
            let child = std::mem::replace(c, Expr { kind: ExprKind::Bool(false), pos: c.pos });
            match self.hoist_expr(child, &mut hoisted, false, depth + 1) {
                Ok(v) => *c = v,
                Err(d) => fail = Some(d),
            }
        });
        if let Some(d) = fail { return Err(d); }
        out.extend(hoisted);
        // `await task { }`: start first, then await its Future.
        let at = e.pos;
        if let ExprKind::Await(op) = &mut e.kind {
            if matches!(op.kind, ExprKind::SpawnTask { detached: false, .. }) {
                let op_pos = op.pos;
                let spawn = std::mem::replace(&mut **op, Expr { kind: ExprKind::Bool(false), pos: op_pos });
                let (name, var) = self.temp(spawn.pos);
                out.push(Stmt::Bind { kind: BindKind::Val, name, ty: None, init: spawn, pos: at });
                **op = var;
            }
        }
        let suspends = matches!(e.kind, ExprKind::Await(_) | ExprKind::SpawnTask { detached: false, .. });
        if suspends && !root_ok {
            let (name, var) = self.temp(e.pos);
            let pos = e.pos;
            out.push(Stmt::Bind { kind: BindKind::Val, name, ty: None, init: e, pos });
            return Ok(var);
        }
        Ok(e)
    }
}

// ---------- helpers ----------

type SuspLambdaBind = (String, Pos, Vec<(String, Option<Type>)>, Option<Type>, Vec<Stmt>, bool);

/// Remove `val f = susp |..| -> ..` bindings (any nesting), returning them.
fn take_susp_lambda_binds(body: &mut Vec<Stmt>, out: &mut Vec<SuspLambdaBind>, depth: usize) -> R<()> {
    if depth > MAX_DEPTH {
        return Err(err(Pos { line: 1, col: 1 }, "E_NESTING", "suspending lambda nesting limit exceeded"));
    }
    let mut kept = Vec::with_capacity(body.len());
    for mut s in std::mem::take(body) {
        if let Stmt::Bind { kind, name, init: Expr { kind: ExprKind::SuspLambda { params, ret, body: lb }, .. }, pos, .. } = &mut s {
            if out.iter().any(|(n, ..)| n == name) {
                return Err(err(*pos, "E_DUPLICATE", format!("suspending lambda '{name}' is bound twice")));
            }
            out.push((name.clone(), *pos, std::mem::take(params), *ret, std::mem::take(lb), *kind == BindKind::Let));
            continue;
        }
        for nested in nested_bodies_mut(&mut s) {
            take_susp_lambda_binds(nested, out, depth + 1)?;
        }
        kept.push(s);
    }
    *body = kept;
    Ok(())
}

fn nested_bodies_mut(s: &mut Stmt) -> Vec<&mut Vec<Stmt>> {
    match s {
        Stmt::StateWrite { body, .. } | Stmt::While { body, .. } | Stmt::Loop { body, .. } |
        Stmt::ForIn { body, .. } | Stmt::Block(body) | Stmt::Defer { body, .. } |
        Stmt::Recover { body, .. } | Stmt::UsingTelemetry { body, .. } | Stmt::For { body, .. } => vec![body],
        Stmt::Try { body, catch, finally, .. } => {
            let mut v = vec![body];
            if let Some((_, c)) = catch { v.push(c); }
            if let Some(f) = finally { v.push(f); }
            v
        }
        Stmt::If { arms, otherwise, .. } => {
            let mut v: Vec<&mut Vec<Stmt>> = arms.iter_mut().map(|(_, b)| b).collect();
            if let Some(o) = otherwise { v.push(o); }
            v
        }
        Stmt::Match { arms, .. } => arms.iter_mut().map(|a| &mut a.body).collect(),
        _ => Vec::new(),
    }
}

/// Deep mutable walk over every expression, entering task bodies.
fn walk_exprs_mut(body: &mut [Stmt], f: &mut dyn FnMut(&mut Expr)) {
    for s in body.iter_mut() {
        map_stmt_exprs(s, &mut |e| walk_expr_mut(e, f));
        for nested in nested_bodies_mut(s) {
            walk_exprs_mut(nested, f);
        }
    }
}

fn walk_expr_mut(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    f(e);
    if let ExprKind::Task { body, .. } = &mut e.kind {
        walk_exprs_mut(body, f);
        return;
    }
    map_child_exprs(e, &mut |c| walk_expr_mut(c, f));
}

fn collect_calls(body: &mut [Stmt], name: &str, out: &mut Vec<Vec<Expr>>) {
    walk_exprs_mut(body, &mut |e| if let ExprKind::Call(n, args) = &e.kind {
        if n == name { out.push(args.clone()); }
    });
}

fn rewrite_lambda_uses(body: &mut [Stmt], name: &str, synthetic: &str, extra: &[Expr], escaped: &mut Option<Pos>) {
    walk_exprs_mut(body, &mut |e| match &mut e.kind {
        ExprKind::Call(n, args) if n == name => {
            *n = synthetic.to_string();
            args.extend(extra.iter().cloned());
        }
        ExprKind::Var(n) if n == name => { escaped.get_or_insert(e.pos); }
        _ => {}
    });
}

fn returns_value(body: &[Stmt]) -> bool {
    let mut found = false;
    visit_stmts(body, &mut |s| if matches!(s, Stmt::Return(Some(_), _)) { found = true; });
    found
}

fn collect_bindings(body: &[Stmt], env: &mut BTreeMap<String, Option<Binding>>,
    rets: &BTreeMap<(String, usize), Type>, depth: usize) -> R<()> {
    let mut binds = Vec::new();
    visit_stmts(body, &mut |s| if let Stmt::Bind { kind, name, ty, init, .. } = s {
        binds.push((name.clone(), *kind == BindKind::Let, *ty, init.clone()));
    });
    let _ = depth;
    for (name, mutable, ty, init) in binds {
        let ty = ty.or_else(|| infer(&init, env, rets));
        let next = Binding { ty, mutable };
        match env.get(&name) {
            None => { env.insert(name, Some(next)); }
            Some(Some(prev)) if prev.ty == next.ty && prev.mutable == next.mutable => {}
            Some(_) => { env.insert(name, None); }
        }
    }
    Ok(())
}

fn infer(e: &Expr, env: &BTreeMap<String, Option<Binding>>,
    rets: &BTreeMap<(String, usize), Type>) -> Option<Type> {
    Some(match &e.kind {
        ExprKind::Int(_) => Type::Int,
        ExprKind::Float(_) => Type::Float,
        ExprKind::Bool(_) => Type::Bool,
        ExprKind::Str(_) => Type::Str,
        ExprKind::Var(n) => return env.get(n).copied().flatten().and_then(|b| b.ty),
        ExprKind::Ascribe(_, t, _) => *t,
        ExprKind::Call(name, args) => return rets.get(&(name.clone(), args.len())).copied()
            .filter(|t| *t != Type::Void),
        ExprKind::Await(op) => match &op.kind {
            ExprKind::Call(name, args) => return rets.get(&(name.clone(), args.len())).copied(),
            _ => match infer(op, env, rets)? { Type::FutureInt => Type::Int, _ => return None },
        },
        ExprKind::Task { detached: false, .. } | ExprKind::SpawnTask { detached: false, .. } => Type::FutureInt,
        ExprKind::Unary(UnOp::Not, _) => Type::Bool,
        ExprKind::Unary(_, a) => return infer(a, env, rets),
        ExprKind::Binary(op, a, b) => match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge |
            BinOp::And | BinOp::Or => Type::Bool,
            _ => {
                let (l, r) = (infer(a, env, rets)?, infer(b, env, rets)?);
                if l == Type::Float || r == Type::Float { Type::Float }
                else if l == Type::Str || r == Type::Str { Type::Str }
                else { l }
            }
        },
        _ => return None,
    })
}

fn mentions_suspension(body: &[Stmt]) -> bool { first_suspension(body).is_some() }

fn first_suspension(body: &[Stmt]) -> Option<Pos> {
    let mut found = None;
    visit_stmts(body, &mut |s| {
        if found.is_none() {
            for_stmt_exprs(s, &mut |e| if found.is_none() { found = find_expr(e, &is_suspension, 0); });
        }
    });
    found
}

fn is_suspension(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Await(_) | ExprKind::Task { .. } | ExprKind::SpawnTask { .. })
}

/// Read-only, iterative-depth-bounded search. Past the bound the walk stops
/// (the parser already caps expression depth far below it).
fn find_expr(e: &Expr, pred: &dyn Fn(&Expr) -> bool, depth: usize) -> Option<Pos> {
    if pred(e) { return Some(e.pos); }
    if depth > MAX_DEPTH * 4 { return None; }
    let mut found = None;
    for_child_exprs(e, &mut |c| if found.is_none() { found = find_expr(c, pred, depth + 1); });
    found
}

fn contains_await(e: &Expr) -> bool {
    find_expr(e, &|x| matches!(x.kind, ExprKind::Await(_)), 0).is_some()
}

fn stmt_contains_await(s: &Stmt) -> bool {
    let mut found = false;
    visit_stmts(std::slice::from_ref(s), &mut |st| {
        if !found { for_stmt_exprs(st, &mut |e| if !found { found = contains_await(e); }); }
    });
    found
}

fn for_stmt_exprs(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    match s {
        Stmt::Bind { init, .. } => f(init),
        Stmt::Assign { value, .. } => f(value),
        Stmt::FieldAssign { target, value, .. } => { f(target); f(value); }
        Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Raise(e, _) | Stmt::Yield(e, _) |
        Stmt::YieldFrom(e, _) | Stmt::Return(Some(e), _) => f(e),
        Stmt::Defer { args, .. } | Stmt::Recover { args, .. } => for a in args { f(a); },
        Stmt::If { arms, .. } => for (c, _) in arms { f(c); },
        Stmt::While { cond, .. } => f(cond),
        Stmt::For { cond, .. } => if let Some(c) = cond { f(c); },
        Stmt::ForIn { source, .. } => f(source),
        Stmt::Match { subject, arms, .. } => {
            f(subject);
            for a in arms { if let Some(g) = &a.guard { f(g); } }
        }
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Block(_) |
        Stmt::StateWrite { .. } | Stmt::Try { .. } | Stmt::Loop { .. } |
        Stmt::UsingTelemetry { .. } => {}
    }
}

fn for_child_exprs(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    match &e.kind {
        ExprKind::Unary(_, a) | ExprKind::Trap(a) | ExprKind::Ownership(_, _, a) |
        ExprKind::Ascribe(a, _, _) | ExprKind::Propagate(a) | ExprKind::Member(a, _) |
        ExprKind::Await(a) => f(a),
        ExprKind::Binary(_, a, b) => { f(a); f(b); }
        ExprKind::Ternary(c, a, b) => { f(c); f(a); f(b); }
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) |
        ExprKind::SpawnTask { args, .. } => for a in args { f(a); },
        ExprKind::MethodCall(recv, _, args) => { f(recv); for a in args { f(a); } }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) |
        ExprKind::Var(_) | ExprKind::SelfRef | ExprKind::StateRead(_) | ExprKind::Spawn(_) |
        ExprKind::Lambda { .. } | ExprKind::Task { .. } | ExprKind::Duration(_) |
        ExprKind::SuspLambda { .. } => {}
    }
}

/// Names used (read or assigned) and bound inside a task body.
fn uses_in_stmts(body: &[Stmt], used: &mut BTreeSet<String>,
    assigned: &mut BTreeSet<String>, bound: &mut BTreeSet<String>) {
    visit_stmts(body, &mut |s| {
        match s {
            Stmt::Bind { name, .. } => { bound.insert(name.clone()); }
            Stmt::Assign { name, .. } => { assigned.insert(name.clone()); used.insert(name.clone()); }
            Stmt::ForIn { name, .. } => { bound.insert(name.clone()); }
            Stmt::Try { catch: Some((name, _)), .. } => { bound.insert(name.clone()); }
            _ => {}
        }
        for_stmt_exprs(s, &mut |e| vars_in(e, used));
    });
}

fn vars_in(e: &Expr, used: &mut BTreeSet<String>) {
    if let ExprKind::Var(n) = &e.kind { used.insert(n.clone()); }
    // A nested task body is captured through this body: its free names
    // must be captured here too, so they can be forwarded.
    if let ExprKind::Task { body, .. } = &e.kind {
        let (mut inner, mut assigned, mut bound) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        uses_in_stmts(body, &mut inner, &mut assigned, &mut bound);
        used.extend(inner.into_iter().filter(|n| !bound.contains(n)));
    }
    for_child_exprs(e, &mut |c| vars_in(c, used));
}

fn stmt_pos(s: &Stmt) -> Pos {
    match s {
        Stmt::Bind { pos, .. } | Stmt::Assign { pos, .. } | Stmt::StateWrite { pos, .. } |
        Stmt::FieldAssign { pos, .. } | Stmt::Throw(_, pos) | Stmt::Raise(_, pos) |
        Stmt::Try { pos, .. } | Stmt::Defer { pos, .. } | Stmt::Recover { pos, .. } |
        Stmt::Yield(_, pos) | Stmt::YieldFrom(_, pos) | Stmt::Return(_, pos) |
        Stmt::If { pos, .. } | Stmt::While { pos, .. } | Stmt::Loop { pos, .. } |
        Stmt::For { pos, .. } | Stmt::ForIn { pos, .. } | Stmt::Break(pos) |
        Stmt::Continue(pos) | Stmt::Match { pos, .. } | Stmt::UsingTelemetry { pos, .. } => *pos,
        Stmt::Expr(e) => e.pos,
        Stmt::Block(b) => b.first().map(stmt_pos).unwrap_or(Pos { line: 1, col: 1 }),
    }
}

/// Pre-order visit of every statement, including nested bodies.
fn visit_stmts(body: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for s in body {
        f(s);
        match s {
            Stmt::StateWrite { body, .. } | Stmt::While { body, .. } | Stmt::Loop { body, .. } |
            Stmt::ForIn { body, .. } | Stmt::Block(body) | Stmt::Defer { body, .. } |
            Stmt::Recover { body, .. } | Stmt::UsingTelemetry { body, .. } => visit_stmts(body, f),
            Stmt::For { init, update, body, .. } => {
                if let Some(i) = init { visit_stmts(std::slice::from_ref(&**i), f); }
                if let Some(u) = update { visit_stmts(std::slice::from_ref(&**u), f); }
                visit_stmts(body, f);
            }
            Stmt::Try { body, catch, finally, .. } => {
                visit_stmts(body, f);
                if let Some((_, c)) = catch { visit_stmts(c, f); }
                if let Some(fin) = finally { visit_stmts(fin, f); }
            }
            Stmt::If { arms, otherwise, .. } => {
                for (_, b) in arms { visit_stmts(b, f); }
                if let Some(o) = otherwise { visit_stmts(o, f); }
            }
            Stmt::Match { arms, .. } => for a in arms { visit_stmts(&a.body, f); },
            _ => {}
        }
    }
}

/// Apply `f` to the expressions owned directly by one statement (not to
/// expressions inside nested statement bodies).
fn map_stmt_exprs(s: &mut Stmt, f: &mut dyn FnMut(&mut Expr)) {
    match s {
        Stmt::Bind { init, .. } => f(init),
        Stmt::Assign { value, .. } => f(value),
        Stmt::FieldAssign { target, value, .. } => { f(target); f(value); }
        Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Raise(e, _) | Stmt::Yield(e, _) |
        Stmt::YieldFrom(e, _) | Stmt::Return(Some(e), _) => f(e),
        Stmt::Defer { args, .. } | Stmt::Recover { args, .. } => for a in args { f(a); },
        Stmt::If { arms, .. } => for (c, _) in arms { f(c); },
        Stmt::While { cond, .. } => f(cond),
        Stmt::For { cond, .. } => if let Some(c) = cond { f(c); },
        Stmt::ForIn { source, .. } => f(source),
        Stmt::Match { subject, arms, .. } => {
            f(subject);
            for a in arms { if let Some(g) = &mut a.guard { f(g); } }
        }
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Block(_) |
        Stmt::StateWrite { .. } | Stmt::Try { .. } | Stmt::Loop { .. } |
        Stmt::UsingTelemetry { .. } => {}
    }
}

/// Apply `f` to each direct child expression of `e`. Task bodies and lambda
/// bodies are separate callables and are not entered.
fn map_child_exprs(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    match &mut e.kind {
        ExprKind::Unary(_, a) | ExprKind::Trap(a) | ExprKind::Ownership(_, _, a) |
        ExprKind::Ascribe(a, _, _) | ExprKind::Propagate(a) | ExprKind::Member(a, _) |
        ExprKind::Await(a) => f(a),
        ExprKind::Binary(_, a, b) => { f(a); f(b); }
        ExprKind::Ternary(c, a, b) => { f(c); f(a); f(b); }
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) |
        ExprKind::SpawnTask { args, .. } => for a in args { f(a); },
        ExprKind::MethodCall(recv, _, args) => { f(recv); for a in args { f(a); } }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) |
        ExprKind::Var(_) | ExprKind::SelfRef | ExprKind::StateRead(_) | ExprKind::Spawn(_) |
        ExprKind::Lambda { .. } | ExprKind::Task { .. } | ExprKind::Duration(_) |
        ExprKind::SuspLambda { .. } => {}
    }
}
