//! Executable, deliberately narrow closure conversion for fnc(int) => int.
 //!
 //! Converts source callable values to a private class-backed closure profile.
 //! Generated constructors snapshot immutable integer captures by value, while
 //! the synthesized call receiver dispatches to a closed set of lambda bodies.
 //! This is real guest-Wasm execution via checked class/method lowering, NOT the
 //! final typed call_indirect ABI, reference-counted cleanup, or actor transport.
use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use super::callable_ir::CallableSignature;
use std::collections::{BTreeMap, BTreeSet};
type R<T> = Result<T, Diagnostic>;

const CLASS: &str = "$owned_lambda_int_int";
const CAP_LIMIT: usize = 4;
const SITE_LIMIT: usize = 32;
const DEPTH: usize = 80;

fn fail(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn e(kind: ExprKind, pos: Pos) -> Expr { Expr { kind, pos } }
fn v(name: &str, pos: Pos) -> Expr { e(ExprKind::Var(name.to_string()), pos) }
fn int(n: u64, pos: Pos) -> Expr { e(ExprKind::Int(Some(n)), pos) }
fn self_field(field: &str, pos: Pos) -> Expr {
    e(ExprKind::Member(Box::new(e(ExprKind::SelfRef, pos)), field.to_owned()), pos)
}

#[derive(Clone)]
struct Site {
    tag: u64,
    captures: Vec<String>,
    original_params: Vec<String>,
    expression: Expr,
    pos: Pos,
}
fn collect(expr: &Expr, found: &mut BTreeSet<String>, depth: usize) -> R<()> {
    if depth > DEPTH { return Err(fail(expr.pos, "E_NESTING", "closure expression too deep")); }
    match &expr.kind {
        ExprKind::Var(name) => { found.insert(name.clone()); }
        ExprKind::Int(_) => {},
        ExprKind::Unary(UnOp::Plus | UnOp::Neg, x) => collect(x, found, depth + 1)?,
        ExprKind::Binary(_, a, b) => {
            collect(a, found, depth + 1)?;
            collect(b, found, depth + 1)?;
        }
        _ => return Err(fail(expr.pos, "E_UNSUPPORTED",
            "owned int closures admit pure integer expressions, variables and integer operators only")),
    }
    Ok(())
}
fn lift_body(expr: &mut Expr, params: &[String], captures: &[String], depth: usize) -> R<()> {
    if depth > DEPTH { return Err(fail(expr.pos, "E_NESTING", "closure rewrite too deep")); }
    match &mut expr.kind {
        ExprKind::Var(n) => {
            if *n == params[0] {
                *n = "value".to_string();
            } else if let Some(i) = captures.iter().position(|x| x == n) {
                *expr = self_field(&format!("capture_{i}"), expr.pos);
            } else {
                return Err(fail(expr.pos, "E_CAPTURE", "unresolved closed-over binding"));
            }
        }
        ExprKind::Unary(_, x) => lift_body(x, params, captures, depth + 1)?,
        ExprKind::Binary(_, a, b) => {
            lift_body(a, params, captures, depth + 1)?;
            lift_body(b, params, captures, depth + 1)?;
        }
        ExprKind::Int(_) => {},
        _ => return Err(fail(expr.pos, "E_UNSUPPORTED", "closure body not admitted")),
    }
    Ok(())
}

fn contains_call(expr: &Expr, named: &BTreeSet<String>, depth: usize) -> bool {
    if depth > DEPTH { return false; }
    match &expr.kind {
        ExprKind::Call(name, args) =>
            named.contains(name) || args.iter().any(|x| contains_call(x, named, depth + 1)),
        ExprKind::Binary(_, a, b) =>
            contains_call(a, named, depth + 1) || contains_call(b, named, depth + 1),
        ExprKind::Unary(_, x) | ExprKind::Trap(x) | ExprKind::Propagate(x) =>
            contains_call(x, named, depth + 1),
        ExprKind::MethodCall(x, _, args) =>
            contains_call(x, named, depth + 1) || args.iter().any(|a| contains_call(a, named, depth + 1)),
        _ => false,
    }
}
fn should_lower(p: &Program) -> bool {
    p.funcs.iter().any(|f| {
        let named: BTreeSet<String> = f.params.iter()
            .filter(|x| matches!(x.ty, Type::Callable(_)))
            .map(|x| x.name.clone())
            .chain(f.body.iter().filter_map(|stmt| match stmt {
                Stmt::Bind { name, ty: Some(Type::Callable(_)), .. } => Some(name.clone()),
                _ => None,
            })).collect();
        f.body.iter().any(|stmt| match stmt {
            Stmt::Return(Some(Expr { kind: ExprKind::Lambda { .. }, .. }), _)
                if matches!(f.ret, Type::Callable(_)) => true,
            Stmt::Return(Some(x), _) | Stmt::Expr(x) =>
                contains_call(x, &named, 0),
            Stmt::Bind { init, .. } => contains_call(init, &named, 0),
            _ => false,
        })
    })
}
fn rewrite_expr(
    expr: &mut Expr, available: &BTreeMap<String, (Type, bool)>,
    callables: &BTreeSet<String>, sites: &mut Vec<Site>, depth: usize,
) -> R<bool> {
    if depth > DEPTH { return Err(fail(expr.pos, "E_NESTING", "closure conversion depth exceeded")); }
    match &mut expr.kind {
        ExprKind::Lambda { params, return_type, body } => {
            if params.len() != 1 || return_type.as_deref().is_some_and(|x| x != "int") {
                return Err(fail(expr.pos, "E_ARITY", "first owned callable profile needs one int parameter and int result"));
            }
            let expression = match body {
                LambdaBody::Expression(x) => (**x).clone(),
                _ => return Err(fail(expr.pos, "E_UNSUPPORTED",
                    "owned callable v1 requires a pure expression lambda body")),
            };
            if sites.len() >= SITE_LIMIT {
                return Err(fail(expr.pos, "E_SOURCE_LIMIT", "too many owned closure sites"));
            }
            let mut referenced = BTreeSet::new();
            collect(&expression, &mut referenced, 0)?;
            referenced.remove(&params[0]);
            if referenced.len() > CAP_LIMIT {
                return Err(fail(expr.pos, "E_SOURCE_LIMIT", "too many captured integer bindings"));
            }
            for name in &referenced {
                match available.get(name) {
                    Some((Type::Int, true)) => {},
                    _ => return Err(fail(expr.pos, "E_CAPTURE",
                        format!("owned closure cannot capture mutable, borrowed or non-int binding '{name}'"))),
                }
            }
            let captures: Vec<String> = referenced.into_iter().collect();
            let tag = sites.len() as u64;
            sites.push(Site {
                tag, captures: captures.clone(), original_params: params.clone(),
                expression, pos: expr.pos,
            });
            let mut args = vec![int(tag, expr.pos)];
            args.extend(captures.iter().map(|name| v(name, expr.pos)));
            while args.len() <= CAP_LIMIT { args.push(int(0, expr.pos)); }
            expr.kind = ExprKind::New(CLASS.to_string(), args);
            Ok(true)
        }
        ExprKind::Call(name, args) => {
            for arg in args.iter_mut() { rewrite_expr(arg, available, callables, sites, depth + 1)?; }
            if callables.contains(name) {
                let recv = e(ExprKind::Var(name.clone()), expr.pos);
                expr.kind = ExprKind::MethodCall(Box::new(recv), "call".to_string(), args.clone());
            }
            Ok(false)
        }
        ExprKind::Binary(_, a, b) => {
            rewrite_expr(a, available, callables, sites, depth + 1)?;
            rewrite_expr(b, available, callables, sites, depth + 1)?;
            Ok(false)
        }
        ExprKind::Unary(_, x) | ExprKind::Trap(x) | ExprKind::Propagate(x) =>
            rewrite_expr(x, available, callables, sites, depth + 1),
        ExprKind::Ternary(a,b,c) => {
            rewrite_expr(a, available, callables, sites, depth + 1)?;
            rewrite_expr(b, available, callables, sites, depth + 1)?;
            rewrite_expr(c, available, callables, sites, depth + 1)?;
            Ok(false)
        }
        ExprKind::MethodCall(base, _, args) => {
            rewrite_expr(base, available, callables, sites, depth + 1)?;
            for arg in args { rewrite_expr(arg, available, callables, sites, depth + 1)?; }
            Ok(false)
        }
        ExprKind::New(_, args) | ExprKind::Array(args) => {
            for arg in args { rewrite_expr(arg, available, callables, sites, depth + 1)?; }
            Ok(false)
        }
        ExprKind::Member(base, _) => rewrite_expr(base, available, callables, sites, depth + 1),
        _ => Ok(false),
    }
}
fn closure_class(sites: &[Site], pos: Pos) -> R<ClassDecl> {
    let mut fields = vec!["selector".to_string()];
    fields.extend((0..CAP_LIMIT).map(|i| format!("capture_{i}")));
    let fields_decl = fields.iter().map(|name| Field {
        name: name.clone(), pos, ty: Type::Int,
        public: false, mode: FieldMode::Constructor, init: int(0, pos),
    }).collect();
    let ctor = Constructor {
        pos,
        params: fields.iter().map(|name| Param {
            name: format!("initial_{name}"), ty: Type::Int, mutable: false, pos,
        }).collect(),
        body: fields.iter().map(|name| Stmt::FieldAssign {
            target: e(ExprKind::SelfRef, pos), field: name.clone(),
            value: v(&format!("initial_{name}"), pos), pos,
        }).collect(),
    };
    let mut arms = Vec::new();
    for site in sites {
        let mut body = site.expression.clone();
        lift_body(&mut body, &site.original_params, &site.captures, 0)?;
        let guard = e(ExprKind::Binary(BinOp::Eq,
            Box::new(self_field("selector", site.pos)),
            Box::new(int(site.tag, site.pos))), site.pos);
        arms.push((guard, vec![Stmt::Return(Some(body), site.pos)]));
    }
    // An invalid selector cannot be created from source because the synthetic
    // class name is not lexable. Keep a structurally total method for Wasm.
    let fallback = vec![Stmt::Return(Some(int(0, pos)), pos)];
    let method = Method {
        name: "call".to_string(), pos, public: true, is_static: false,
        params: vec![Param {
            name: "value".to_string(), ty: Type::Int, mutable: false, pos,
        }],
        ret: Type::Int,
        body: vec![Stmt::If { arms, otherwise: Some(fallback), pos }],
    };
    Ok(ClassDecl {
        name: CLASS.to_string(), pos, public: false,
        fields: fields_decl, ctor: Some(ctor), methods: vec![method],
    })
}
pub fn lower(input: &Program) -> R<Program> {
    if !should_lower(input) { return Ok(input.clone()); }
    let pos = Pos { line: 1, col: 1 };
    if input.classes.iter().any(|c| c.name == CLASS)
        || input.funcs.iter().any(|f| f.name == CLASS)
        || input.actors.iter().any(|a| a.name == CLASS) {
        return Err(fail(pos, "E_DUPLICATE", "reserved generated closure class name collision"));
    }
    for (i, (name, at)) in input.type_names.iter().enumerate() {
        if name.starts_with("fnc(") {
            let signature = CallableSignature::from_source_type(
                Type::Callable(i as u32), &input.type_names, *at)?;
            if signature.params != [Type::Int] || signature.result != Type::Int {
                return Err(fail(*at, "E_UNSUPPORTED_TYPE",
                    "owned closure conversion initially supports fnc(int) => int only"));
            }
        }
    }
    let mut p = input.clone();
    let new_type = p.type_names.len() as u32;
    p.type_names.push((CLASS.to_string(), pos));
    for f in &mut p.funcs {
        if matches!(f.ret, Type::Callable(_)) { f.ret = Type::Named(new_type); }
        for param in &mut f.params {
            if matches!(param.ty, Type::Callable(_)) { param.ty = Type::Named(new_type); }
        }
    }
    let mut sites = Vec::new();
    let function_returns: BTreeSet<String> = input.funcs.iter()
        .filter(|f| matches!(f.ret, Type::Callable(_))).map(|f| f.name.clone()).collect();
    for f in &mut p.funcs {
        let mut available: BTreeMap<String, (Type, bool)> = f.params.iter().map(|param|
            (param.name.clone(), (param.ty, !param.mutable))).collect();
        let mut callables: BTreeSet<String> = f.params.iter()
            .filter(|x| x.ty == Type::Named(new_type))
            .map(|x| x.name.clone()).collect();
        for stmt in &mut f.body {
            match stmt {
                Stmt::Bind { kind, name, ty, init, .. } => {
                    let originally_callable = matches!(ty, Some(Type::Callable(_)));
                    if originally_callable { *ty = Some(Type::Named(new_type)); }
                    let lambda = rewrite_expr(init, &available, &callables, &mut sites, 0)?;
                    let from_callable_function = match &init.kind {
                        ExprKind::Call(callee, _) => function_returns.contains(callee),
                        _ => false,
                    };
                    let inherits_callable = match &init.kind {
                        ExprKind::Var(other) => callables.contains(other),
                        _ => false,
                    };
                    if lambda || originally_callable || from_callable_function || inherits_callable {
                        callables.insert(name.clone());
                    } else if *kind == BindKind::Val {
                        if matches!(&init.kind, ExprKind::Int(_)) {
                            available.insert(name.clone(), (Type::Int, true));
                        } else if let ExprKind::Var(n) = &init.kind {
                            if available.get(n).is_some_and(|(t, yes)| *t == Type::Int && *yes) {
                                available.insert(name.clone(), (Type::Int, true));
                            }
                        }
                    }
                    if let Some(Type::Int) = ty {
                        available.insert(name.clone(), (Type::Int, *kind != BindKind::Let));
                    }
                }
                Stmt::Return(Some(expr), _) | Stmt::Expr(expr) | Stmt::Throw(expr, _) => {
                    rewrite_expr(expr, &available, &callables, &mut sites, 0)?;
                }
                Stmt::Return(None, _) => {},
                _ => return Err(fail(f.pos, "E_UNSUPPORTED",
                    "owned closure conversion currently supports flat function bodies only")),
            }
        }
    }
    if sites.is_empty() {
        return Err(fail(pos, "E_UNSUPPORTED", "no executable owned closure site"));
    }
    p.classes.push(closure_class(&sites, pos)?);
    Ok(p)
}
