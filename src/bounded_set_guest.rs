//! Restricted, non-escaping immutable local Set<int> factory profile.
//! The original parsed arguments are evaluated by the ordinary typed guest
//! constructor, exactly once and in order. Duplicate membership/cardinality
//! are computed from real stored values at runtime, not by a host evaluator.
//! General collection types, aliases, mutation, export and actor transport
//! remain outside this profile and use the existing fail-closed checker.
use crate::ast::*;
use crate::diag::Pos;
use std::collections::BTreeSet;

const MAX_SLOTS: usize = 8;
const CLASS: &str = "__atto_local_set_v1";

fn expr(kind: ExprKind, pos: Pos) -> Expr { Expr { kind, pos } }
fn int(n: u64, pos: Pos) -> Expr { expr(ExprKind::Int(Some(n)), pos) }
fn var(n: &str, pos: Pos) -> Expr { expr(ExprKind::Var(n.into()), pos) }
fn binary(op: BinOp, a: Expr, b: Expr, pos: Pos) -> Expr {
    expr(ExprKind::Binary(op, Box::new(a), Box::new(b)), pos)
}
fn choose(c: Expr, a: Expr, b: Expr, pos: Pos) -> Expr {
    expr(ExprKind::Ternary(Box::new(c), Box::new(a), Box::new(b)), pos)
}
fn slot(i: usize, pos: Pos) -> Expr {
    expr(ExprKind::Member(Box::new(expr(ExprKind::SelfRef, pos)), format!("v{i}")), pos)
}
fn param(n: String, pos: Pos) -> Param {
    Param { name: n, pos, ty: Type::Int, mutable: false, omittable: false,
        none_default: None, omission_default: None }
}

// Closed scalar expressions only. A set may occur exclusively as a query
// receiver in the final result, never as a value, argument or field.
fn scalar(e: &Expr, locals: &BTreeSet<String>, owner: Option<&str>, queried: &mut bool) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Bool(_) => true,
        ExprKind::Var(n) => locals.contains(n),
        ExprKind::Unary(_, a) => scalar(a, locals, owner, queried),
        ExprKind::Binary(_, a, b) => scalar(a, locals, owner, queried)
            && scalar(b, locals, owner, queried),
        ExprKind::Ternary(c, a, b) => scalar(c, locals, owner, queried)
            && scalar(a, locals, owner, queried) && scalar(b, locals, owner, queried),
        ExprKind::MethodCall(base, name, args)
            if matches!(&base.kind, ExprKind::Var(n) if Some(n.as_str()) == owner) =>
        {
            let valid = match name.as_str() {
                "len" => args.is_empty(),
                "contains" => args.len() == 1 && scalar(&args[0], locals, None, queried),
                _ => false,
            };
            if valid { *queried = true; }
            valid
        }
        _ => false,
    }
}

fn storage(n: usize, pos: Pos) -> ClassDecl {
    let mut fields = Vec::new();
    let mut params = Vec::new();
    let mut body = Vec::new();
    for i in 0..n {
        fields.push(Field { name: format!("v{i}"), pos, ty: Type::Int,
            visibility: FieldVisibility::Private, mode: FieldMode::Final,
            binding: FieldBinding::Final, referent_mutable: false, init: None });
        params.push(param(format!("p{i}"), pos));
        body.push(Stmt::FieldAssign { target: expr(ExprKind::SelfRef, pos),
            field: format!("v{i}"), value: var(&format!("p{i}"), pos), pos });
    }
    let mut count = int(0, pos);
    let mut contains = expr(ExprKind::Bool(false), pos);
    for i in 0..n {
        // Count a slot iff no earlier slot has the same scalar value.
        let mut unique = expr(ExprKind::Bool(true), pos);
        for j in 0..i {
            unique = choose(binary(BinOp::Eq, slot(i, pos), slot(j, pos), pos),
                expr(ExprKind::Bool(false), pos), unique, pos);
        }
        count = binary(BinOp::Add, count,
            choose(unique, int(1, pos), int(0, pos), pos), pos);
        contains = choose(binary(BinOp::Eq, slot(i, pos), var("needle", pos), pos),
            expr(ExprKind::Bool(true), pos), contains, pos);
    }
    ClassDecl { name: CLASS.into(), pos, public: false, state_container: false,
        fields, ctor: Some(Constructor { pos, params, body }),
        methods: vec![
            Method { name: "len".into(), pos, public: true, is_static: false,
                params: vec![], ret: Type::Int, body: vec![Stmt::Return(Some(count), pos)] },
            Method { name: "contains".into(), pos, public: true, is_static: false,
                params: vec![param("needle".into(), pos)], ret: Type::Bool,
                body: vec![Stmt::Return(Some(contains), pos)] },
        ] }
}

/// Rewrite only one completely closed main: scalar declarations, one inferred
/// non-const factory, then a scalar return querying that factory. Never
/// partially mutate an unsupported program. The normal checker and emitters
/// still own all type, ownership, heap and runtime validation.
pub(crate) fn lower(p: &mut Program) {
    if !p.classes.is_empty() || !p.enums.is_empty() || !p.actors.is_empty()
        || p.actor_interface.is_some() || !p.type_names.is_empty()
        || !p.array_types.is_empty() || p.funcs.len() != 1 { return; }
    let f = &mut p.funcs[0];
    if f.name != "main" || !f.params.is_empty() || f.ret != Type::Int
        || f.is_generator || f.is_pure || f.declared_effects.is_some() { return; }
    let Some((last, prefix)) = f.body.split_last() else { return; };
    let Stmt::Return(Some(result), _) = last else { return; };
    let mut locals = BTreeSet::new();
    let mut factory: Option<(usize, &str, usize, Pos)> = None;
    let mut queried = false;
    for (index, s) in prefix.iter().enumerate() {
        let Stmt::Bind { kind, name, ty, init, pos } = s else { return; };
        if factory.is_some() || name == "Set" || name == CLASS || locals.contains(name) { return; }
        if let ExprKind::MethodCall(base, method, args) = &init.kind {
            if method == "of" && matches!(&base.kind, ExprKind::Var(n) if n == "Set")
                && *kind != BindKind::Const && ty.is_none()
                && (1..=MAX_SLOTS).contains(&args.len())
                && args.iter().all(|a| scalar(a, &locals, None, &mut queried))
            {
                factory = Some((index, name.as_str(), args.len(), *pos));
                continue;
            }
        }
        if ty.is_some_and(|t| t != Type::Int && t != Type::Bool)
            || !scalar(init, &locals, None, &mut queried) { return; }
        locals.insert(name.clone());
    }
    let Some((index, owner, count, pos)) = factory else { return; };
    if !scalar(result, &locals, Some(owner), &mut queried) || !queried { return; }
    let Stmt::Bind { init, .. } = &mut f.body[index] else { return; };
    let ExprKind::MethodCall(_, _, args) = &mut init.kind else { return; };
    init.kind = ExprKind::New(CLASS.into(), std::mem::take(args));
    p.classes.push(storage(count, pos));
}
