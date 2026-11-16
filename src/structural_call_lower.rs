//! Int-only, by-value structural-call lowering for the restricted browser frontend.
//!
//! This is NOT the general borrowed structural ABI. One top-level ~Class formal
//! is expanded into a finite collection of immutable integer values. Each call
//! projects the public integer fields from the *actual* source class using the
//! production member checker, which resolves source-specific memory offsets.
//! The class object is NOT cast, moved, passed, or exposed to the callee.
//!
//! Crucially the rewrite happens before the normal type checker, and every
//! unsupported structural formal/call remains fail-closed: we cannot silently
//! lower a genuine structural loan as a nominal class pointer.
//!
//! The first admitted executable profile requires a local or typed parameter
//! holding an ordinary class instance. Computed, named, async, actor-state and
//! dynamic-map arguments require a stronger loan/snapshot IR.

use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use std::collections::BTreeMap;
use std::cell::Cell;

type R<T> = Result<T, Diagnostic>;

const MAX_PROJECTED_INTS: usize = 8;

struct Profile {
    name: String,
    formal: String,
    fields: Vec<String>,
    replacements: BTreeMap<String, String>,
    pos: Pos,
    rewrites: Cell<usize>,
}

fn unsupported(pos: Pos, detail: &str) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED",
        format!("readonly structural call requires checked scalar projection: {detail}"))
}

fn ordinary_class<'a>(p: &'a Program, name: &str) -> Option<&'a ClassDecl> {
    p.classes.iter().find(|c| c.name == name && !c.state_container)
}

fn class_of_named<'a>(p: &'a Program, ty: Type) -> Option<&'a str> {
    if let Type::Named(i) = ty {
        let (name, _) = p.type_names.get(i as usize)?;
        ordinary_class(p, name).map(|_| name.as_str())
    } else {
        None
    }
}

fn profile(p: &Program) -> R<Option<Profile>> {
    // Only a top-level single-argument ~Class fnc is executable in this
    // stage; method, constructor and actor receiver ABIs stay explicitly gated.
    let mut candidates = p.funcs.iter().filter(|f| f.params.iter().any(|q| q.structural));
    let Some(f) = candidates.next() else { return Ok(None); };
    if candidates.next().is_some() {
        return Err(unsupported(f.pos, "multiple structural functions need a type-indexed dispatch table"));
    }
    if f.is_generator || f.params.len() != 1 || !f.params[0].structural {
        return Err(unsupported(f.pos, "only a single ~Class argument on a non-generator top-level fnc"));
    }
    let param = &f.params[0];
    if param.mutable {
        return Err(Diagnostic::new(param.pos, "E_BORROW",
            "structural parameters must be read-only"));
    }
    let name = class_of_named(p, param.ty)
        .ok_or_else(|| unsupported(param.pos,
            "anonymous shapes, scalar formals and unresolved class shapes require typed descriptor IR"))?;
    let source = ordinary_class(p, name).unwrap();
    let public: Vec<_> = source.fields.iter().filter(|q| q.public).collect();
    if public.is_empty() || public.len() > MAX_PROJECTED_INTS {
        return Err(unsupported(param.pos,
            "structural class must expose between one and eight public int fields"));
    }
    let mut fields = Vec::new();
    let mut replacements = BTreeMap::new();
    for field in public {
        if field.ty != Type::Int || field.mode == FieldMode::ActorState {
            return Err(unsupported(param.pos,
                "initial executable structural calls admit public int fields only"));
        }
        let hidden = format!("$shape_{}_{}", param.name, field.name);
        fields.push(field.name.clone());
        replacements.insert(field.name.clone(), hidden);
    }
    Ok(Some(Profile {
        name: f.name.clone(), formal: param.name.clone(),
        fields, replacements, pos: param.pos, rewrites: Cell::new(0)
    }))
}

/// Substitute only checked field reads. Bare use of the receiver and all
/// writes/escapes are rejected; no method or getter can gain a hidden loan.
fn replace_formal_expr(e: &mut Expr, s: &Profile) -> R<()> {
    if let ExprKind::Member(base, field) = &e.kind {
        if matches!(&base.kind, ExprKind::Var(n) if n == &s.formal) {
            let target = s.replacements.get(field)
                .ok_or_else(|| unsupported(e.pos,
                    "formal class member is not a public int field"))?;
            e.kind = ExprKind::Var(target.clone());
            return Ok(());
        }
    }
    match &mut e.kind {
        ExprKind::Var(n) if *n == s.formal =>
            Err(Diagnostic::new(e.pos, "E_BORROW",
                "structural receiver cannot escape or be passed by value; project a public field")),
        ExprKind::Unary(_, x) | ExprKind::Trap(x) | ExprKind::Propagate(x) |
        ExprKind::Ownership(_, _, x) | ExprKind::Member(x, _) =>
            replace_formal_expr(x, s),
        ExprKind::Binary(_, a, b) => {
            replace_formal_expr(a, s)?;
            replace_formal_expr(b, s)
        }
        ExprKind::Ternary(a, b, c) => {
            replace_formal_expr(a, s)?;
            replace_formal_expr(b, s)?;
            replace_formal_expr(c, s)
        }
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) => {
            for arg in args { replace_formal_expr(arg, s)?; }
            Ok(())
        }
        ExprKind::MethodCall(recv, _, args) => {
            replace_formal_expr(recv, s)?;
            for arg in args { replace_formal_expr(arg, s)?; }
            Ok(())
        }
        ExprKind::Lambda { .. } =>
            Err(unsupported(e.pos, "closure capture requires borrow/lifetime analysis")),
        _ => Ok(())
    }
}

fn replace_formal_stmts(body: &mut [Stmt], s: &Profile) -> R<()> {
    for stmt in body {
        match stmt {
            Stmt::Bind { name, init, pos, .. } => {
                if name == &s.formal {
                    return Err(unsupported(*pos, "a local must not shadow the structural formal"));
                }
                replace_formal_expr(init, s)?;
            }
            Stmt::Assign { name, value, pos } => {
                if name == &s.formal {
                    return Err(Diagnostic::new(*pos, "E_BORROW",
                        "cannot assign a structural formal"));
                }
                replace_formal_expr(value, s)?;
            }
            Stmt::FieldAssign { target, .. } => {
                // Any field assignment to p.x would be a writable structural
                // loan. Detect before substituting the read expression.
                return Err(unsupported(target.pos,
                    "field writes inside a structural function require an exclusive-borrow ABI"));
            }
            Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Yield(e, _) |
            Stmt::YieldFrom(e, _) => replace_formal_expr(e, s)?,
            Stmt::Return(Some(e), _) => replace_formal_expr(e, s)?,
            Stmt::If { arms, otherwise, .. } => {
                for (cond, body) in arms {
                    replace_formal_expr(cond, s)?;
                    replace_formal_stmts(body, s)?;
                }
                if let Some(body) = otherwise { replace_formal_stmts(body, s)?; }
            }
            Stmt::While { cond, body, .. } => {
                replace_formal_expr(cond, s)?;
                replace_formal_stmts(body, s)?;
            }
            Stmt::Loop { body, .. } | Stmt::Block(body) |
            Stmt::StateWrite { body, .. } => replace_formal_stmts(body, s)?,
            Stmt::For { init, cond, update, body, .. } => {
                if let Some(init) = init { replace_formal_stmts(std::slice::from_mut(init.as_mut()), s)?; }
                if let Some(cond) = cond { replace_formal_expr(cond, s)?; }
                if let Some(update) = update { replace_formal_stmts(std::slice::from_mut(update.as_mut()), s)?; }
                replace_formal_stmts(body, s)?;
            }
            Stmt::ForIn { name, source, body, pos } => {
                if name == &s.formal {
                    return Err(unsupported(*pos, "iterator binding shadows structural parameter"));
                }
                replace_formal_expr(source, s)?;
                replace_formal_stmts(body, s)?;
            }
            Stmt::Match { subject, arms, .. } => {
                replace_formal_expr(subject, s)?;
                for arm in arms {
                    match &arm.pattern {
                        Pattern::Bind(n) if n == &s.formal =>
                            return Err(unsupported(arm.pos, "match binder shadows structural parameter")),
                        Pattern::StructuralType { bind: Some(n), .. } |
                        Pattern::StructuralFields { bind: Some(n), .. } if n == &s.formal =>
                            return Err(unsupported(arm.pos, "structural binder shadows formal")),
                        _ => (),
                    }
                    if let Some(g) = &mut arm.guard { replace_formal_expr(g, s)?; }
                    replace_formal_stmts(&mut arm.body, s)?;
                }
            }
            Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => (),
        }
    }
    Ok(())
}

fn actual_type(p: &Program, expr: &Expr, env: &BTreeMap<String, String>) -> Option<String> {
    match &expr.kind {
        ExprKind::New(name, _) if ordinary_class(p, name).is_some() => Some(name.clone()),
        ExprKind::Var(n) => env.get(n).cloned(),
        _ => None,
    }
}

fn actual_fields_compatible(p: &Program, ty: &str, s: &Profile, pos: Pos) -> R<()> {
    let c = ordinary_class(p, ty).ok_or_else(|| unsupported(pos,
        "actual argument must be an ordinary class with a statically known layout"))?;
    for name in &s.fields {
        let field = c.fields.iter().find(|f| f.name == *name)
            .ok_or_else(|| Diagnostic::new(pos, "E_TYPE",
                format!("class '{}' lacks required public structural field '{}'", ty, name)))?;
        if !field.public {
            return Err(Diagnostic::new(pos, "E_PRIVATE",
                format!("field '{}.{}' is not publicly readable", ty, name)));
        }
        if field.ty != Type::Int || field.mode == FieldMode::ActorState {
            return Err(Diagnostic::new(pos, "E_TYPE",
                format!("structural field '{}.{}' must be an ordinary int", ty, name)));
        }
    }
    Ok(())
}

/// Rewrite an original f(source) call into f(source.x, source.y, ...)
/// so normal typed Member lowering proves actual source offsets and ownership.
fn rewrite_calls_expr(
    p: &Program, e: &mut Expr, s: &Profile, env: &BTreeMap<String, String>
) -> R<()> {
    if let ExprKind::Call(name, args) = &e.kind {
        if name == &s.name && args.len() == 1 {
            let arg = &args[0];
            // This profile cannot safely read computed/new temporary twice:
            // a later view IR must evaluate it once and hold a loan.
            let ExprKind::Var(var) = &arg.kind else {
                return Err(unsupported(arg.pos,
                    "structural argument must be a named, live class local or nominal parameter"));
            };
            let ty = env.get(var).ok_or_else(|| unsupported(arg.pos,
                "structural argument must have a statically known nominal class type"))?;
            actual_fields_compatible(p, ty, s, arg.pos)?;
            let expanded = s.fields.iter().map(|name| Expr {
                kind: ExprKind::Member(
                    Box::new(Expr { kind: ExprKind::Var(var.clone()), pos: arg.pos }),
                    name.clone(),
                ),
                pos: arg.pos,
            }).collect();
            e.kind = ExprKind::Call(name.clone(), expanded);
            s.rewrites.set(s.rewrites.get() + 1);
            return Ok(());
        }
    }
    match &mut e.kind {
        ExprKind::Unary(_, x) | ExprKind::Trap(x) | ExprKind::Propagate(x) |
        ExprKind::Ownership(_, _, x) | ExprKind::Member(x, _) =>
            rewrite_calls_expr(p, x, s, env),
        ExprKind::Binary(_, a, b) => {
            rewrite_calls_expr(p, a, s, env)?;
            rewrite_calls_expr(p, b, s, env)
        }
        ExprKind::Ternary(a, b, c) => {
            rewrite_calls_expr(p, a, s, env)?;
            rewrite_calls_expr(p, b, s, env)?;
            rewrite_calls_expr(p, c, s, env)
        }
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) => {
            for arg in args { rewrite_calls_expr(p, arg, s, env)?; }
            Ok(())
        }
        ExprKind::MethodCall(recv, _, args) => {
            rewrite_calls_expr(p, recv, s, env)?;
            for arg in args { rewrite_calls_expr(p, arg, s, env)?; }
            Ok(())
        }
        _ => Ok(())
    }
}

fn rewrite_calls_stmts(
    p: &Program, body: &mut [Stmt], s: &Profile,
    env: &mut BTreeMap<String, String>
) -> R<()> {
    for stmt in body {
        match stmt {
            Stmt::Bind { name, ty, init, .. } => {
                rewrite_calls_expr(p, init, s, env)?;
                let declared = ty.and_then(|t| class_of_named(p, t).map(str::to_owned));
                let inferred = actual_type(p, init, env);
                if let Some(kind) = declared.or(inferred) {
                    env.insert(name.clone(), kind);
                } else {
                    env.remove(name);
                }
            }
            Stmt::Assign { name, value, .. } => {
                rewrite_calls_expr(p, value, s, env)?;
                // The normal checker validates assignments against the old
                // nominal type; keep that stable only if still statically known.
                if let Some(kind) = actual_type(p, value, env) {
                    env.insert(name.clone(), kind);
                } else {
                    env.remove(name);
                }
            }
            Stmt::FieldAssign { target, value, .. } => {
                rewrite_calls_expr(p, target, s, env)?;
                rewrite_calls_expr(p, value, s, env)?;
            }
            Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Yield(e, _) |
            Stmt::YieldFrom(e, _) => rewrite_calls_expr(p, e, s, env)?,
            Stmt::Return(Some(e), _) => rewrite_calls_expr(p, e, s, env)?,
            Stmt::If { arms, otherwise, .. } => {
                for (cond, body) in arms {
                    rewrite_calls_expr(p, cond, s, env)?;
                    rewrite_calls_stmts(p, body, s, &mut env.clone())?;
                }
                if let Some(body) = otherwise {
                    rewrite_calls_stmts(p, body, s, &mut env.clone())?;
                }
            }
            Stmt::While { cond, body, .. } => {
                rewrite_calls_expr(p, cond, s, env)?;
                rewrite_calls_stmts(p, body, s, &mut env.clone())?;
            }
            Stmt::Loop { body, .. } | Stmt::StateWrite { body, .. } |
            Stmt::Block(body) => rewrite_calls_stmts(p, body, s, &mut env.clone())?,
            Stmt::For { init, cond, update, body, .. } => {
                let mut loop_env = env.clone();
                if let Some(init) = init {
                    rewrite_calls_stmts(p, std::slice::from_mut(init.as_mut()), s, &mut loop_env)?;
                }
                if let Some(cond) = cond { rewrite_calls_expr(p, cond, s, &loop_env)?; }
                if let Some(update) = update {
                    rewrite_calls_stmts(p, std::slice::from_mut(update.as_mut()), s, &mut loop_env)?;
                }
                rewrite_calls_stmts(p, body, s, &mut loop_env)?;
            }
            Stmt::ForIn { name, source, body, .. } => {
                rewrite_calls_expr(p, source, s, env)?;
                let mut nested = env.clone();
                nested.remove(name);
                rewrite_calls_stmts(p, body, s, &mut nested)?;
            }
            Stmt::Match { subject, arms, .. } => {
                rewrite_calls_expr(p, subject, s, env)?;
                for arm in arms {
                    let mut nested = env.clone();
                    match &arm.pattern {
                        Pattern::Bind(n) |
                        Pattern::StructuralType { bind: Some(n), .. } |
                        Pattern::StructuralFields { bind: Some(n), .. } => {
                            nested.remove(n);
                        }
                        _ => (),
                    }
                    if let Some(g) = &mut arm.guard {
                        rewrite_calls_expr(p, g, s, &nested)?;
                    }
                    rewrite_calls_stmts(p, &mut arm.body, s, &mut nested)?;
                }
            }
            Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => (),
        }
    }
    Ok(())
}

/// Optional exact-contract transformation. The verifier still owns the
/// unconditional fail-closed gate for all remaining .structural formals.
pub fn lower(p: &Program) -> R<Program> {
    let Some(s) = profile(p)? else { return Ok(p.clone()); };
    let mut out = p.clone();
    let i = out.funcs.iter().position(|f| f.name == s.name && f.params.len() == 1)
        .ok_or_else(|| unsupported(s.pos, "source declaration disappeared"))?;
    // Lower only safe projections. By-value scalar snapshots have no loan to
    // escape and no dynamic cross-class pointer layout in the callee.
    replace_formal_stmts(&mut out.funcs[i].body, &s)?;
    out.funcs[i].params = s.fields.iter().map(|name| Param {
        name: s.replacements[name].clone(),
        ty: Type::Int,
        structural: false,
        mutable: false,
        pos: s.pos,
    }).collect();

    // Verify all known caller sites against the ORIGINAL symbol table and
    // declarations, never the modified function's synthetic signature.
    for f in &mut out.funcs {
        let mut env = BTreeMap::new();
        for param in &f.params {
            if let Some(c) = class_of_named(p, param.ty) {
                env.insert(param.name.clone(), c.to_owned());
            }
        }
        rewrite_calls_stmts(p, &mut f.body, &s, &mut env)?;
    }
    for c in &mut out.classes {
        for m in &mut c.methods {
            let mut env = BTreeMap::new();
            for param in &m.params {
                if let Some(class) = class_of_named(p, param.ty) {
                    env.insert(param.name.clone(), class.to_owned());
                }
            }
            rewrite_calls_stmts(p, &mut m.body, &s, &mut env)?;
        }
        if let Some(ctor) = &mut c.ctor {
            let mut env = BTreeMap::new();
            for param in &ctor.params {
                if let Some(class) = class_of_named(p, param.ty) {
                    env.insert(param.name.clone(), class.to_owned());
                }
            }
            rewrite_calls_stmts(p, &mut ctor.body, &s, &mut env)?;
        }
    }
    for a in &mut out.actors {
        if let Some(r) = &mut a.receive {
            rewrite_calls_stmts(p, &mut r.body, &s, &mut BTreeMap::new())?;
        }
        for m in &mut a.methods {
            let mut env = BTreeMap::new();
            for param in &m.params {
                if let Some(c) = class_of_named(p, param.ty) {
                    env.insert(param.name.clone(), c.to_owned());
                }
            }
            rewrite_calls_stmts(p, &mut m.body, &s, &mut env)?;
        }
    }
    // No checked static call witness: retain the pre-existing unsupported
    // formal, rather than changing the ABI of an otherwise unproven export.
    if s.rewrites.get() == 0 { return Ok(p.clone()); }
    Ok(out)
}
