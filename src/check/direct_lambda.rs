//! Conservative direct-call lowering of capture-free lambdas to typed functions.
//!
//! One immutable local lambda per source function is supported. Direct calls
//! become named private Wasm function calls. This provides real execution
//! without claiming callable values, captured environments or indirect tables.
use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use super::lambda_types::check_noncapturing;
type R<T> = Result<T, Diagnostic>;

struct Direct {
    source: String,
    hidden: String,
    lambda: Expr,
    params: Vec<String>,
    inferred: Option<Vec<Type>>,
    calls: usize,
}
fn error(p: Pos, code: &'static str, m: impl Into<String>) -> Diagnostic {
    Diagnostic::new(p, code, m)
}
fn literal_type(e: &Expr) -> Option<Type> {
    match &e.kind {
        ExprKind::Int(_) => Some(Type::Int),
        ExprKind::Bool(_) => Some(Type::Bool),
        ExprKind::Float(_) => Some(Type::Float),
        ExprKind::Str(_) => Some(Type::Str),
        ExprKind::Unary(UnOp::Plus | UnOp::Neg, inner) if
            matches!(inner.kind, ExprKind::Int(_) | ExprKind::Float(_)) => literal_type(inner),
        _ => None,
    }
}
pub fn lower(input: &Program) -> R<Program> {
    let mut program = input.clone();
    let mut generated = Vec::new();
    for (idx, f) in program.funcs.iter_mut().enumerate() {
        if f.is_generator { continue; }
        let mut direct: Option<Direct> = None;
        let mut body = Vec::new();
        for mut stmt in std::mem::take(&mut f.body) {
            if let Stmt::Bind { kind, name, ty, init, pos } = &stmt {
                if let ExprKind::Lambda { params, .. } = &init.kind {
                    if direct.is_some() {
                        return Err(error(*pos, "E_UNSUPPORTED",
                            "at most one direct lambda per function is supported"));
                    }
                    if *kind != BindKind::Val || ty.is_some() {
                        return Err(error(*pos, "E_UNSUPPORTED",
                            "direct lambdas need an unannotated immutable val binding"));
                    }
                    if f.params.iter().any(|p| p.name == *name) {
                        return Err(error(*pos, "E_DUPLICATE",
                            "lambda name duplicates a function parameter"));
                    }
                    if generated.len() >= 64 {
                        return Err(error(*pos, "E_SOURCE_LIMIT", "too many direct lambdas"));
                    }
                    // Dollar prefix cannot be spelled in user identifiers.
                    direct = Some(Direct {
                        source: name.clone(),
                        hidden: format!("$lambda_{}", idx),
                        lambda: init.clone(),
                        params: params.clone(), inferred: None, calls: 0,
                    });
                    continue;
                }
            }
            if let Some(d) = &mut direct { rewrite_stmt(&mut stmt, d)?; }
            body.push(stmt);
        }
        if let Some(d) = direct {
            if d.calls == 0 {
                return Err(error(d.lambda.pos, "E_UNSUPPORTED",
                    "lambda is not directly invoked; first-class callables remain unsupported"));
            }
            let types = d.inferred.ok_or_else(|| error(d.lambda.pos, "E_INFER",
                "lambda parameter types need a direct invocation"))?;
            let signature = check_noncapturing(&d.lambda, Some(&types), None)?;
            let inner = match d.lambda.kind {
                ExprKind::Lambda { body, .. } => body,
                _ => unreachable!(),
            };
            let lambda_body = match inner {
                LambdaBody::Expression(e) => {
                    let p = e.pos;
                    vec![Stmt::Return(Some(*e), p)]
                }
                LambdaBody::Statement(s) => vec![*s],
                LambdaBody::Block(b) => b,
            };
            generated.push(Func {
                name: d.hidden,
                pos: d.lambda.pos,
                is_generator: false,
                params: d.params.into_iter().zip(types).map(|(name, ty)| Param {
                    name, ty, mutable: false, pos: d.lambda.pos,
                }).collect(),
                ret: signature.result,
                body: lambda_body,
            });
        }
        f.body = body;
    }
    if program.funcs.len().saturating_add(generated.len()) > crate::parser::MAX_FUNCS {
        return Err(error(Pos { line: 1, col: 1 }, "E_SOURCE_LIMIT",
            "compiled lambdas exceed the module function budget"));
    }
    program.funcs.extend(generated);
    Ok(program)
}
fn rewrite_statements(body: &mut [Stmt], d: &mut Direct) -> R<()> {
    for stmt in body { rewrite_stmt(stmt, d)?; }
    Ok(())
}
fn rewrite_stmt(s: &mut Stmt, d: &mut Direct) -> R<()> {
    match s {
        Stmt::Bind { name, init, pos, .. } => {
            if *name == d.source {
                return Err(error(*pos, "E_UNSUPPORTED",
                    "shadowing direct lambda is unsupported"));
            }
            rewrite_expr(init, d)
        }
        Stmt::Assign { name, value, pos } => {
            if *name == d.source {
                return Err(error(*pos, "E_IMMUTABLE",
                    "direct lambda cannot be reassigned"));
            }
            rewrite_expr(value, d)
        }
        Stmt::FieldAssign { target, value, .. } => {
            rewrite_expr(target, d)?;
            rewrite_expr(value, d)
        }
        Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Yield(e, _) | Stmt::YieldFrom(e, _) =>
            rewrite_expr(e, d),
        Stmt::Return(Some(e), _) => rewrite_expr(e, d),
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => Ok(()),
        Stmt::Block(b) | Stmt::Loop { body: b, .. } => rewrite_statements(b, d),
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, d)?;
            rewrite_statements(body, d)
        }
        Stmt::If { arms, otherwise, .. } => {
            for (cond, body) in arms {
                rewrite_expr(cond, d)?;
                rewrite_statements(body, d)?;
            }
            if let Some(other) = otherwise { rewrite_statements(other, d)?; }
            Ok(())
        }
        Stmt::For { init, cond, update, body, .. } => {
            if let Some(x) = init { rewrite_stmt(x, d)?; }
            if let Some(x) = cond { rewrite_expr(x, d)?; }
            if let Some(x) = update { rewrite_stmt(x, d)?; }
            rewrite_statements(body, d)
        }
        Stmt::ForIn { name, source, body, pos } => {
            if *name == d.source {
                return Err(error(*pos, "E_UNSUPPORTED",
                    "for-in may not shadow direct lambda"));
            }
            rewrite_expr(source, d)?;
            rewrite_statements(body, d)
        }
        Stmt::Match { subject, arms, .. } => {
            rewrite_expr(subject, d)?;
            for arm in arms {
                let bound = match &arm.pattern {
                    Pattern::Bind(n) | Pattern::Adt(_, Some(n)) => Some(n),
                    _ => None,
                };
                if bound.is_some_and(|n| *n == d.source) {
                    return Err(error(arm.pos, "E_UNSUPPORTED",
                        "match arm cannot shadow direct lambda"));
                }
                if let Some(guard) = &mut arm.guard { rewrite_expr(guard, d)?; }
                rewrite_statements(&mut arm.body, d)?;
            }
            Ok(())
        }
    }
}
fn rewrite_expr(e: &mut Expr, d: &mut Direct) -> R<()> {
    match &mut e.kind {
        ExprKind::Var(n) if *n == d.source =>
            Err(error(e.pos, "E_UNSUPPORTED",
                "direct lambda cannot escape as a first-class value")),
        ExprKind::Call(name, args) => {
            for a in args.iter_mut() { rewrite_expr(a, d)?; }
            if *name == d.source {
                if args.len() != d.params.len() {
                    return Err(error(e.pos, "E_ARITY", format!(
                        "lambda expects {} arguments, got {}",
                        d.params.len(), args.len())));
                }
                if d.inferred.is_none() {
                    let types = args.iter().map(literal_type).collect::<Option<Vec<_>>>()
                        .ok_or_else(|| error(e.pos, "E_INFER",
                            "first direct call must have scalar literal arguments"))?;
                    d.inferred = Some(types);
                }
                *name = d.hidden.clone();
                d.calls += 1;
            }
            Ok(())
        }
        ExprKind::Unary(_, x) | ExprKind::Trap(x) | ExprKind::Propagate(x) =>
            rewrite_expr(x, d),
        ExprKind::Binary(_, x, y) => {
            rewrite_expr(x, d)?;
            rewrite_expr(y, d)
        }
        ExprKind::Ternary(c, x, y) => {
            rewrite_expr(c, d)?;
            rewrite_expr(x, d)?;
            rewrite_expr(y, d)
        }
        ExprKind::Member(x, _) => rewrite_expr(x, d),
        ExprKind::MethodCall(x, _, args) => {
            rewrite_expr(x, d)?;
            for a in args { rewrite_expr(a, d)?; }
            Ok(())
        }
        ExprKind::Array(args) => {
            for a in args { rewrite_expr(a, d)?; }
            Ok(())
        }
        ExprKind::Lambda { .. } => Err(error(e.pos, "E_UNSUPPORTED",
            "nested or escaping lambda needs closure lowering")),
        _ => Ok(()),
    }
}
