//! First escaping lexical closure runtime profile for the native+wasm32 compiler.
 //! 
//! Converts one immutable `fnc(int)=>int` factory-returned expression lambda
//! into a private nominal class with an owned guest-heap capture environment.
//! Each factory invocation constructs a distinct object. Calls are statically
//! resolved to the generated class's checked method ABI: no function table,
//! untrusted function pointer, borrowed capture or host-managed object.
//!
//! This is intentionally NOT a general first-class callable/indirect ABI.
//! Its unlexable $-prefixed private class is owned by the guest instance's
//! bounded bump heap, which
//! has no independent per-closure free operation. Calls on an instance held
//! after the function returns are safe until that entire guest is destroyed.
use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use super::callable_ir::CallableSignature;
use std::collections::BTreeSet;
type R<T> = Result<T, Diagnostic>;

const MAX_DEPTH: usize = 96;
const MAX_CAPTURES: usize = 16;
fn err(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn expr(kind: ExprKind, pos: Pos) -> Expr { Expr { kind, pos } }
fn member_self(name: &str, pos: Pos) -> Expr {
    expr(ExprKind::Member(Box::new(expr(ExprKind::SelfRef, pos)), name.to_owned()), pos)
}

// The first escape profile admits only effect-free scalar expression bodies.
// Reject rather than silently capture actor capabilities, object borrows,
// nested lambdas, mutable bindings, or expressions with unspecified effects.
fn collect(value: &Expr, arg: &str, params: &[Param],
           captures: &mut BTreeSet<String>, depth: usize) -> R<()> {
    if depth >= MAX_DEPTH { return Err(err(value.pos, "E_NESTING",
        "escaping closure body exceeds capture analysis depth")); }
    match &value.kind {
        ExprKind::Int(_) | ExprKind::Bool(_) => Ok(()),
        ExprKind::Var(name) if name == arg => Ok(()),
        ExprKind::Var(name) => {
            let param = params.iter().find(|p| &p.name == name)
                .ok_or_else(|| err(value.pos, "E_CAPTURE", format!(
                    "escaping closure cannot capture unknown or non-parameter binding '{name}'")))?;
            if param.mutable || param.ty != Type::Int {
                return Err(err(value.pos, "E_CAPTURE", format!(
                    "escaping closure requires immutable int parameter '{name}'")));
            }
            captures.insert(name.clone());
            if captures.len() > MAX_CAPTURES {
                return Err(err(value.pos, "E_SOURCE_LIMIT", "too many escaping closure captures"));
            }
            Ok(())
        }
        ExprKind::Unary(_, inner) => collect(inner, arg, params, captures, depth + 1),
        ExprKind::Binary(_, left, right) => {
            collect(left, arg, params, captures, depth + 1)?;
            collect(right, arg, params, captures, depth + 1)
        }
        ExprKind::Ternary(cond, yes, no) => {
            collect(cond, arg, params, captures, depth + 1)?;
            collect(yes, arg, params, captures, depth + 1)?;
            collect(no, arg, params, captures, depth + 1)
        }
        _ => Err(err(value.pos, "E_UNSUPPORTED",
            "escaping closure body admits only scalar literals, parameters and pure operators")),
    }
}
fn replace_captures(value: &mut Expr, names: &BTreeSet<String>, depth: usize) -> R<()> {
    if depth >= MAX_DEPTH { return Err(err(value.pos, "E_NESTING",
        "escaping closure rewrite nesting exceeded")); }
    match &mut value.kind {
        ExprKind::Var(name) if names.contains(name) => {
            let capture = name.clone();
            *value = member_self(&capture, value.pos);
        }
        ExprKind::Unary(_, inner) => replace_captures(inner, names, depth + 1)?,
        ExprKind::Binary(_, a, b) => {
            replace_captures(a, names, depth + 1)?;
            replace_captures(b, names, depth + 1)?;
        }
        ExprKind::Ternary(a, b, c) => {
            replace_captures(a, names, depth + 1)?;
            replace_captures(b, names, depth + 1)?;
            replace_captures(c, names, depth + 1)?;
        }
        _ => {}
    }
    Ok(())
}
fn is_factory_call(value: &Expr, name: &str) -> bool {
    matches!(&value.kind, ExprKind::Call(target, _) if target == name)
}
// A callable value is admitted only as a distinct immutable local created
// by the known factory. The class is a nominal, move-checked owned value.
// Importantly: a source `fnc(int)=>int` is NOT lowered for arbitrary
// parameters, aliases, classes, actor transport or other closure factories.
fn rewrite_expr(e: &mut Expr, known: &BTreeSet<String>, depth: usize) -> R<()> {
    if depth >= MAX_DEPTH { return Err(err(e.pos, "E_NESTING",
        "escaping closure call rewrite nesting exceeded")); }
    match &mut e.kind {
        ExprKind::Call(name, args) => {
            for a in args.iter_mut() { rewrite_expr(a, known, depth + 1)?; }
            if known.contains(name) {
                let binding = name.clone();
                let args = std::mem::take(args);
                e.kind = ExprKind::MethodCall(
                    Box::new(expr(ExprKind::Var(binding), e.pos)),
                    "call".to_owned(), args,
                );
            }
        }
        ExprKind::Unary(_, a) | ExprKind::Trap(a) | ExprKind::Propagate(a) =>
            rewrite_expr(a, known, depth + 1)?,
        ExprKind::Binary(_, a, b) => {
            rewrite_expr(a, known, depth + 1)?;
            rewrite_expr(b, known, depth + 1)?;
        }
        ExprKind::Ternary(c, a, b) => {
            rewrite_expr(c, known, depth + 1)?;
            rewrite_expr(a, known, depth + 1)?;
            rewrite_expr(b, known, depth + 1)?;
        }
        ExprKind::Member(a, _) => rewrite_expr(a, known, depth + 1)?,
        ExprKind::MethodCall(a, _, args) => {
            rewrite_expr(a, known, depth + 1)?;
            for x in args { rewrite_expr(x, known, depth + 1)?; }
        }
        ExprKind::Array(args) | ExprKind::New(_, args) => {
            for x in args { rewrite_expr(x, known, depth + 1)?; }
        }
        _ => {}
    }
    Ok(())
}
fn nested_stmt(s: &mut Stmt, known: &BTreeSet<String>, depth: usize) -> R<()> {
    if depth >= MAX_DEPTH { return Err(err(stmt_pos(s), "E_NESTING",
        "escaping closure statement rewrite nesting exceeded")); }
    match s {
        Stmt::Bind { name, init, pos, .. } => {
            if known.contains(name) { return Err(err(*pos, "E_CAPTURE",
                "nested binding shadows an escaping callable")); }
            rewrite_expr(init, known, depth + 1)?;
        }
        Stmt::Assign { name, value, pos } => {
            if known.contains(name) { return Err(err(*pos, "E_IMMUTABLE",
                "escaping callable binding cannot be reassigned")); }
            rewrite_expr(value, known, depth + 1)?;
        }
        Stmt::FieldAssign { target, value, .. } => {
            rewrite_expr(target, known, depth + 1)?;
            rewrite_expr(value, known, depth + 1)?;
        }
        Stmt::Expr(x) | Stmt::Throw(x, _) | Stmt::Yield(x, _) | Stmt::YieldFrom(x, _)
        | Stmt::Return(Some(x), _) => rewrite_expr(x, known, depth + 1)?,
        Stmt::If { arms, otherwise, .. } => {
            for (condition, body) in arms {
                rewrite_expr(condition, known, depth + 1)?;
                for stmt in body { nested_stmt(stmt, known, depth + 1)?; }
            }
            if let Some(body) = otherwise {
                for stmt in body { nested_stmt(stmt, known, depth + 1)?; }
            }
        }
        Stmt::For { init, cond, update, body, .. } => {
            if let Some(s) = init { nested_stmt(s, known, depth + 1)?; }
            if let Some(e) = cond { rewrite_expr(e, known, depth + 1)?; }
            if let Some(s) = update { nested_stmt(s, known, depth + 1)?; }
            for s in body { nested_stmt(s, known, depth + 1)?; }
        }
        Stmt::ForIn { name, source, body, pos } => {
            if known.contains(name) { return Err(err(*pos, "E_CAPTURE",
                "for-in binder shadows escaping callable")); }
            rewrite_expr(source, known, depth + 1)?;
            for s in body { nested_stmt(s, known, depth + 1)?; }
        }
        Stmt::Match { subject, arms, .. } => {
            rewrite_expr(subject, known, depth + 1)?;
            for arm in arms {
                let name = match &arm.pattern {
                    Pattern::Bind(n) | Pattern::Adt(_, Some(n)) => Some(n),
                    _ => None
                };
                if name.is_some_and(|n| known.contains(n)) {
                    return Err(err(arm.pos, "E_CAPTURE",
                        "match binder shadows escaping callable"));
                }
                if let Some(guard) = &mut arm.guard {
                    rewrite_expr(guard, known, depth + 1)?;
                }
                for stmt in &mut arm.body { nested_stmt(stmt, known, depth + 1)?; }
            }
        }
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, known, depth + 1)?;
            for s in body { nested_stmt(s, known, depth + 1)?; }
        }
        Stmt::Block(body) | Stmt::Loop { body, .. } => {
            for s in body { nested_stmt(s, known, depth + 1)?; }
        }
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => {}
    }
    Ok(())
}
fn stmt_pos(s: &Stmt) -> Pos {
    match s {
        Stmt::Bind { pos, .. } | Stmt::Assign { pos, .. } |
        Stmt::FieldAssign { pos, .. } | Stmt::If { pos, .. } |
        Stmt::While { pos, .. } | Stmt::Loop { pos, .. } |
        Stmt::For { pos, .. } | Stmt::ForIn { pos, .. } |
        Stmt::Match { pos, .. } | Stmt::Throw(_, pos) |
        Stmt::Yield(_, pos) | Stmt::YieldFrom(_, pos) |
        Stmt::Return(_, pos) | Stmt::Break(pos) |
        Stmt::Continue(pos) => *pos,
        Stmt::Block(body) => body.first().map_or(Pos { line: 1, col: 1 }, stmt_pos),
        Stmt::Expr(e) => e.pos,
    }
}

pub fn lower(source: &Program) -> R<Program> {
    let mut p = source.clone();
    let factories: Vec<usize> = p.funcs.iter().enumerate().filter_map(|(i, f)|
        matches!(f.ret, Type::Callable(_)).then_some(i)).collect();
    if factories.is_empty() { return Ok(p); }
    if factories.len() != 1 {
        return Err(err(p.funcs[factories[1]].pos, "E_UNSUPPORTED",
            "the first escaping closure profile permits only one factory signature per module"));
    }
    let index = factories[0];
    let f = &p.funcs[index];
    let signature = CallableSignature::from_source_type(f.ret, &p.type_names, f.pos)?;
    if signature.params != [Type::Int] || signature.result != Type::Int {
        return Err(err(f.pos, "E_UNSUPPORTED_TYPE",
            "escaping closure v1 admits only fnc(int)=>int"));
    }
    if f.is_generator || f.name == "main" {
        return Err(err(f.pos, "E_UNSUPPORTED", "escaping callable factory must be an ordinary non-entry function"));
    }
    let Stmt::Return(Some(value), _) = f.body.first().ok_or_else(|| err(f.pos,
        "E_UNSUPPORTED", "escaping callable factory requires a single returned lambda"))?
    else { return Err(err(f.pos, "E_UNSUPPORTED",
        "escaping callable factory requires a single returned lambda")); };
    if f.body.len() != 1 { return Err(err(f.pos, "E_UNSUPPORTED",
        "escaping callable factory must contain exactly one return statement")); }
    let ExprKind::Lambda { params, return_type, body: LambdaBody::Expression(body) } = &value.kind
    else { return Err(err(value.pos, "E_UNSUPPORTED",
        "escaping callable factory requires an expression-bodied lambda")); };
    if params.len() != 1 || return_type.as_deref().is_some_and(|t| t != "int") {
        return Err(err(value.pos, "E_TYPE",
            "escaping callable lambda requires exactly one int argument and int result"));
    }
    let callback_arg = params[0].clone();
    let source_name = f.name.clone();
    let pos = value.pos;
    let class_name = format!("$atto_escaping_closure_{index}");
    if p.classes.iter().any(|c| c.name == class_name)
        || p.actors.iter().any(|c| c.name == class_name)
        || p.enums.iter().any(|c| c.name == class_name)
        || p.funcs.iter().any(|c| c.name == class_name) {
        return Err(err(f.pos, "E_DUPLICATE",
            "reserved escaping closure class name already declared"));
    }
    let mut used = BTreeSet::new();
    collect(body, &callback_arg, &f.params, &mut used, 0)?;
    if used.is_empty() {
        return Err(err(pos, "E_UNSUPPORTED",
            "noncapturing callable values need a separate first-class ABI"));
    }
    let ordered: Vec<String> = f.params.iter().filter(|p| used.contains(&p.name))
        .map(|p| p.name.clone()).collect();
    let factory_return_type = f.ret;
    let class_type = p.type_names.len() as u32;
    p.type_names.push((class_name.clone(), pos));
    let mut body_expr = body.as_ref().clone();
    replace_captures(&mut body_expr, &used, 0)?;
    let fields = ordered.iter().map(|name| Field {
        name: name.clone(), pos, ty: Type::Int, public: false,
        mode: FieldMode::Constructor,
        init: expr(ExprKind::Int(Some(0)), pos),
    }).collect();
    let ctor_params = ordered.iter().map(|name| Param {
        name: name.clone(), ty: Type::Int, mutable: false, pos,
    }).collect();
    let ctor_body = ordered.iter().map(|name| Stmt::FieldAssign {
        target: expr(ExprKind::SelfRef, pos),
        field: name.clone(),
        value: expr(ExprKind::Var(name.clone()), pos),
        pos,
    }).collect();
    let generated_class = ClassDecl {
        name: class_name.clone(), pos, public: false,
        fields,
        ctor: Some(Constructor { pos, params: ctor_params, body: ctor_body }),
        methods: vec![Method {
            name: "call".to_owned(), pos, public: true, is_static: false,
            params: vec![Param { name: callback_arg, ty: Type::Int, mutable: false, pos }],
            ret: Type::Int,
            body: vec![Stmt::Return(Some(body_expr), pos)],
        }],
    };
    p.classes.push(generated_class);
    p.funcs[index].ret = Type::Named(class_type);
    p.funcs[index].body = vec![Stmt::Return(Some(expr(
        ExprKind::New(class_name.clone(), ordered.iter()
            .map(|name| expr(ExprKind::Var(name.clone()), pos)).collect()),
        pos,
    )), pos)];

    // Only direct immutable locals produced by this specific factory are
    // callable by source `local(value)`. All other callable storage,
    // parameters, aliasing and transport still go through the old fail gate.
    for (i, function) in p.funcs.iter_mut().enumerate() {
        if i == index { continue; }
        let mut known = BTreeSet::<String>::new();
        for statement in &mut function.body {
            if let Stmt::Bind { name, ty, init, pos, kind } = statement {
                if known.contains(name) { return Err(err(*pos, "E_DUPLICATE",
                    "escaping callable local is redeclared")); }
                let factory = is_factory_call(init, &source_name);
                rewrite_expr(init, &known, 0)?;
                if factory {
                    if *kind != BindKind::Val {
                        return Err(err(*pos, "E_UNSUPPORTED",
                            "escaping callable local must be immutable val"));
                    }
                    if let Some(t) = ty {
                        if *t != factory_return_type {
                            return Err(err(*pos, "E_TYPE",
                                "escaping callable local type must match factory signature"));
                        }
                        *ty = Some(Type::Named(class_type));
                    }
                    known.insert(name.clone());
                }
            } else {
                nested_stmt(statement, &known, 0)?;
            }
        }
    }
    Ok(p)
}
