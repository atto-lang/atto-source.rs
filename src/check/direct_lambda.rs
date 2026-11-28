//! Conservative direct-call lowering of bounded, immutable scalar-capturing lambdas.
//!
//! Up to eight independent immutable local lambdas per source function are supported. Direct calls
//! become named private Wasm function calls with frozen scalar capture parameters.
//! Callable values, escaping closures, mutable captures and indirect tables
//! remain unsupported.
use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use super::callable_ir::{CallableSignature, CallableTarget, TypedCallable};
use std::collections::{BTreeMap, BTreeSet};
type R<T> = Result<T, Diagnostic>;

#[derive(Clone)]
struct Capture { source: String, snapshot: String, ty: Type }

struct Direct {
    source: String,
    hidden: String,
    lambda: Expr,
    params: Vec<String>,
    inferred: Option<Vec<Type>>,
    declared: Option<CallableSignature>,
    captures: Vec<Capture>,
    calls: usize,
    expression_depth: usize,
    statement_depth: usize,
}
const MAX_REWRITE_DEPTH: usize = 128;
const MAX_DIRECT_LAMBDAS_PER_FUNCTION: usize = 8;
const MAX_DIRECT_LAMBDAS_PER_MODULE: usize = 64;
fn error(p: Pos, code: &'static str, m: impl Into<String>) -> Diagnostic {
    Diagnostic::new(p, code, m)
}
fn literal_type(e: &Expr) -> Option<Type> {
    match &e.kind {
        ExprKind::Int(_) => Some(Type::Int),
        ExprKind::Bool(_) => Some(Type::Bool),
        ExprKind::Float(_) => Some(Type::Float),
        ExprKind::Str(_) => Some(Type::Str),
        ExprKind::Ascribe(inner, ty, _) => literal_type(inner).filter(|found| found == ty),
        ExprKind::Unary(UnOp::Plus | UnOp::Neg, inner) if
            matches!(inner.kind, ExprKind::Int(_) | ExprKind::Float(_)) => literal_type(inner),
        _ => None,
    }
}
pub fn lower(input: &Program) -> R<Program> {
    // A hand-built AST cannot introduce an executable declaration for the
    // temporary apply operand marker. Source identifiers cannot spell it.
    let marker = crate::parser::APPLY_TUPLE_LITERAL_NAME;
    if input.funcs.iter().any(|f| f.name == marker)
        || input.classes.iter().any(|c| c.name == marker)
        || input.enums.iter().any(|e| e.name == marker)
        || input.actors.iter().any(|a| a.name == marker)
    {
        return Err(error(Pos { line: 1, col: 1 }, "E_DUPLICATE",
            "reserved compiler tuple operand marker"));
    }
    let mut program = input.clone();
    let mut generated = Vec::new();
    for (idx, f) in program.funcs.iter_mut().enumerate() {
        if f.is_generator { continue; }
        let mut direct = Vec::<Direct>::new();
        // Discover every same-function direct-lambda name before lowering.
        // Looking only at already visited declarations would miss a forward
        // reference inside an earlier lambda body, potentially binding the
        // callee to an unrelated top-level function after helper generation.
        let lambda_names = f.body.iter().filter_map(|stmt| match stmt {
            Stmt::Bind { name, init, .. }
                if matches!(&init.kind, ExprKind::Lambda { .. }) =>
                Some(name.clone()),
            _ => None,
        }).collect::<BTreeSet<_>>();
        // Keep the top-level lexical binding ledger before desugaring removes
        // the lambda declaration. The ordinary checker would otherwise miss
        // an earlier same-scope declaration with the identical name.
        let mut bindings_before_lambda = BTreeSet::<String>::new();
        // Immutable scalar parameters are stable capture sources. A val
        // initialized from one of these parameters is also an immutable
        // snapshot (unlike a let alias, mutable parameter or owned object).
        // Keep this deliberately narrower than general closure capture:
        // no reference/heap handle or mutable value may escape the frame.
        let mut scalar_immutables = f.params.iter()
            .filter(|p| !p.mutable && matches!(p.ty, Type::Int | Type::Bool | Type::Float))
            .map(|p| (p.name.clone(), p.ty))
            .collect::<BTreeMap<String, Type>>();
        let mut body = Vec::new();
        for mut stmt in std::mem::take(&mut f.body) {
            if let Stmt::Bind { kind, name, ty, init, pos } = &stmt {
                if let ExprKind::Lambda { params, .. } = &init.kind {
                    if direct.len() >= MAX_DIRECT_LAMBDAS_PER_FUNCTION {
                        return Err(error(*pos, "E_SOURCE_LIMIT", "too many direct lambdas in one function"));
                    }
                    let declared = if let Some(t @ Type::Callable(_)) = ty {
                        let want = CallableSignature::from_source_type(
                            *t, &input.type_names, *pos)?;
                        let checked = CallableSignature::from_noncapturing(
                            init, Some(&want.params), Some(want.result))?;
                        if checked != want {
                            return Err(error(*pos, "E_TYPE", "typed lambda differs from declaration"));
                        }
                        if !matches!(kind, BindKind::Val | BindKind::Const) {
                            return Err(error(*pos, "E_UNSUPPORTED",
                                "mutable callable values require verified Wasm indirect dispatch"));
                        }
                        Some(want)
                    } else {
                        if *kind != BindKind::Val || ty.is_some() {
                            return Err(error(*pos, "E_UNSUPPORTED",
                                "direct lambdas need an unannotated immutable val binding"));
                        }
                        None
                    };
                    if bindings_before_lambda.contains(name) {
                        return Err(error(*pos, "E_DUPLICATE",
                            "direct lambda duplicates an existing local binding"));
                    }
                    if f.params.iter().any(|p| p.name == *name) {
                        return Err(error(*pos, "E_DUPLICATE",
                            "lambda name duplicates a function parameter"));
                    }
                    if generated.len().saturating_add(direct.len()) >= MAX_DIRECT_LAMBDAS_PER_MODULE {
                        return Err(error(*pos, "E_SOURCE_LIMIT", "too many direct lambdas"));
                    }
                    // The private name is reserved by source grammar but a
                    // checker caller may construct a Program directly. Reject
                    // collisions instead of letting a forged declaration
                    // impersonate the compiled callback.
                    // Preserve the existing first-helper ABI/name. Siblings
                    // receive a distinct private name and capture snapshot key.
                    let ordinal = direct.len();
                    let hidden = if ordinal == 0 {
                        format!("$lambda_{}", idx)
                    } else {
                        format!("$lambda_{}_{}", idx, ordinal)
                    };
                    if input.funcs.iter().any(|f| f.name == hidden)
                        || input.classes.iter().any(|c| c.name == hidden)
                        || input.enums.iter().any(|e| e.name == hidden)
                        || input.actors.iter().any(|a| a.name == hidden) {
                        return Err(error(*pos, "E_DUPLICATE",
                            "compiler-generated lambda name collides with a declaration"));
                    }
                    let mut free = BTreeSet::new();
                    if let ExprKind::Lambda { body: lambda_body, .. } = &init.kind {
                        // A complete, depth-bounded call graph preflight covers
                        // expression, statement and block bodies, including
                        // nested branches, handlers, cleanup and match arms.
                        // Without it, an unlowered local sibling/forward call
                        // might accidentally resolve to a top-level homonym.
                        let local_call = match lambda_body {
                            LambdaBody::Expression(expr) =>
                                calls_any_named(expr, &lambda_names, 0)?,
                            LambdaBody::Statement(stmt) =>
                                calls_any_named_stmt(stmt, &lambda_names, 0)?,
                            LambdaBody::Block(stmts) =>
                                calls_any_named_statements(stmts, &lambda_names, 0)?,
                        };
                        if local_call {
                            return Err(error(*pos, "E_UNSUPPORTED",
                                "local lambda calls and self-recursion require verified closure environment linking"));
                        }
                        // Capture lifting stays deliberately limited to
                        // expression bodies. Statement and block bodies retain
                        // the independent noncapturing type/effect checker.
                        if let LambdaBody::Expression(expr) = lambda_body {
                            free_names(expr, &mut free, 0)?;
                        }
                    }
                    for p in params { free.remove(p); }
                    let mut captures = Vec::new();
                    for candidate in free {
                        if let Some(&ty) = scalar_immutables.get(&candidate) {
                            if captures.len() == 8 || params.len() + captures.len() >= crate::parser::MAX_PARAMS {
                                return Err(error(*pos, "E_SOURCE_LIMIT",
                                    "direct scalar lambda capture budget exceeded"));
                            }
                            captures.push(Capture {
                                source: candidate,
                                snapshot: if ordinal == 0 {
                                    format!("$capture_{}_{}", idx, captures.len())
                                } else {
                                    format!("$capture_{}_{}_{}", idx, ordinal, captures.len())
                                },
                                ty,
                            });
                        }
                    }
                    if declared.is_some() && !captures.is_empty() {
                        return Err(error(*pos, "E_CAPTURE", "typed closed callable cannot capture enclosing state"));
                    }
                    // Snapshot at the lexical definition point, not at a call
                    // inside a shadowing block. Values are immutable scalars.
                    for c in &captures {
                        body.push(Stmt::Bind {
                            kind: BindKind::Val, name: c.snapshot.clone(),
                            ty: Some(c.ty),
                            init: Expr { kind: ExprKind::Var(c.source.clone()), pos: *pos },
                            pos: *pos,
                        });
                    }
                    direct.push(Direct {
                        source: name.clone(),
                        hidden,
                        lambda: init.clone(),
                        params: params.clone(), inferred: declared.as_ref().map(|sig| sig.params.clone()),
                        declared, captures, calls: 0,
                        expression_depth: 0, statement_depth: 0,
                    });
                    bindings_before_lambda.insert(name.clone());
                    continue;
                }
            }
            for d in &mut direct { rewrite_stmt(&mut stmt, d)?; }
            if let Stmt::Bind { kind, name, init, .. } = &stmt {
                bindings_before_lambda.insert(name.clone());
                if *kind == BindKind::Val {
                    // A copied immutable scalar remains immutable even when
                    // its initializer is a parameter rather than a literal.
                    // Record only verified source types, never guessed types
                    // for calls, heap objects, mutable locals or ownership ops.
                    let scalar_ty = literal_type(init).or_else(|| match &init.kind {
                        ExprKind::Var(source) => scalar_immutables.get(source).copied(),
                        _ => None,
                    });
                    if let Some(ty @ (Type::Int | Type::Bool | Type::Float)) = scalar_ty {
                        scalar_immutables.insert(name.clone(), ty);
                    }
                }
            }
            body.push(stmt);
        }
        for d in direct {
            if d.calls == 0 {
                return Err(error(d.lambda.pos, "E_UNSUPPORTED",
                    "unused lambdas are not executable; first-class callables require verified indirect dispatch"));
            }
            let types = d.inferred.ok_or_else(|| error(d.lambda.pos, "E_INFER",
                "lambda parameter types need a direct invocation"))?;
            let mut lifted = d.lambda.clone();
            let mut all_types = types.clone();
            if let ExprKind::Lambda { params, .. } = &mut lifted.kind {
                for c in &d.captures {
                    params.push(c.source.clone());
                    all_types.push(c.ty);
                }
            }
            let signature = CallableSignature::from_noncapturing(&lifted, Some(&all_types),
                d.declared.as_ref().map(|sig| sig.result))?;
            if d.declared.as_ref().is_some_and(|want| *want != signature) {
                return Err(error(d.lambda.pos, "E_TYPE", "lowered callable signature differs from declaration"));
            }
            let callable = TypedCallable::checked(
                signature,
                CallableTarget::Direct {
                    function_index: (input.funcs.len() + generated.len()) as u32,
                },
                d.lambda.pos,
            )?;
            callable.check_arguments(&all_types, d.lambda.pos)?;
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
                is_suspending: false,
                is_pure: true,
                declared_effects: None,
                params: d.params.into_iter().chain(
                    d.captures.into_iter().map(|c| c.source)
                ).zip(all_types).map(|(name, ty)| Param {
                    name, ty, mutable: false, pos: d.lambda.pos,
                }).collect(),
                ret: callable.signature().result,
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
    if d.statement_depth >= MAX_REWRITE_DEPTH {
        return Err(error(stmt_pos(s), "E_NESTING",
            "direct lambda statement rewrite nesting limit exceeded"));
    }
    d.statement_depth += 1;
    let result = rewrite_stmt_inner(s, d);
    d.statement_depth -= 1;
    result
}
fn rewrite_stmt_inner(s: &mut Stmt, d: &mut Direct) -> R<()> {
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
        Stmt::StateWrite { pos, .. } => Err(error(*pos, "E_UNSUPPORTED",
            "direct lambda rewriting cannot cross a protected state loan")),
        Stmt::UsingTelemetry { alias, body, pos, .. } => {
            if *alias == d.source {
                return Err(error(*pos, "E_TELEMETRY_ALIAS",
                    "span alias cannot shadow a direct lambda"));
            }
            rewrite_statements(body, d)
        }
        Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Raise(e, _) | Stmt::Yield(e, _) | Stmt::YieldFrom(e, _) =>
            rewrite_expr(e, d),
        Stmt::Try { body, catch, finally, .. } => {
            rewrite_statements(body, d)?;
            if let Some((name, handler)) = catch {
                if *name == d.source {
                    return Err(error(d.lambda.pos, "E_UNSUPPORTED", "catch binding cannot shadow direct lambda"));
                }
                rewrite_statements(handler, d)?;
            }
            if let Some(cleanup) = finally { rewrite_statements(cleanup, d)?; }
            Ok(())
        }
        // Cleanup/recovery bodies run in their own lexical scope. They are
        // intentionally not rewritten as ordinary direct-call sites. A call
        // to a local lambda from either body must fail closed; otherwise the
        // source name could silently resolve to an unrelated global function
        // after the local lambda declaration has been erased.
        Stmt::Defer { args, body, pos, .. } |
        Stmt::Recover { args, body, pos, .. } => {
            let names = BTreeSet::from([d.source.clone()]);
            if calls_any_named_statements(body, &names, 0)? {
                return Err(error(*pos, "E_UNSUPPORTED",
                    "direct lambda calls inside deferred or recovery handlers need closure environment lowering"));
            }
            // Registration arguments are evaluated once in the caller and
            // still receive normal direct-call and ownership validation.
            for arg in args { rewrite_expr(arg, d)?; }
            Ok(())
        }
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
    if d.expression_depth >= MAX_REWRITE_DEPTH {
        return Err(error(e.pos, "E_NESTING",
            "direct lambda expression rewrite nesting limit exceeded"));
    }
    d.expression_depth += 1;
    let result = rewrite_expr_inner(e, d);
    d.expression_depth -= 1;
    result
}
fn rewrite_expr_inner(e: &mut Expr, d: &mut Direct) -> R<()> {
    // Resolve only the lexically tracked immutable callable. Ordinary
    // nominal methods named call remain ordinary MethodCall expressions.
    // Normalize before the existing checked direct-call rewrite, preserving
    // argument order, arity, signature, captures and the private target.
    if let ExprKind::MethodCall(receiver, member, args) = &mut e.kind {
        if matches!(&receiver.kind, ExprKind::Var(name) if *name == d.source) {
            if member == "call" {
                e.kind = ExprKind::Call(d.source.clone(), std::mem::take(args));
            } else if member == "apply" {
                if d.declared.is_none() {
                    return Err(error(e.pos, "E_UNSUPPORTED",
                        "apply requires a closed source-typed callable in this profile"));
                }
                if args.len() != 1 {
                    return Err(error(e.pos, "E_ARITY", "apply expects exactly one tuple"));
                }
                let operand = args.pop().expect("arity checked");
                let ExprKind::Call(marker, elements) = operand.kind else {
                    return Err(error(e.pos, "E_UNSUPPORTED",
                        "apply requires an immediate parenthesized scalar tuple literal"));
                };
                if marker != crate::parser::APPLY_TUPLE_LITERAL_NAME
                    || elements.iter().any(|arg| literal_type(arg).is_none())
                {
                    return Err(error(e.pos, "E_UNSUPPORTED",
                        "apply requires an immediate parenthesized scalar tuple literal"));
                }
                e.kind = ExprKind::Call(d.source.clone(), elements);
            }
        }
    }
    match &mut e.kind {
        ExprKind::Await(inner) => rewrite_expr(inner, d),
        ExprKind::SpawnTask { args, .. } => {
            for a in args.iter_mut() { rewrite_expr(a, d)?; }
            Ok(())
        }
        ExprKind::Duration(_) => Ok(()),
        ExprKind::Task { .. } | ExprKind::SuspLambda { .. } => Err(error(e.pos, "E_UNSUPPORTED",
            "task { } and suspending lambdas must be desugared before direct lambda conversion")),
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
                // Every call uses the definition-site snapshot, even if
                // the source name is shadowed in the caller's nested block.
                args.extend(d.captures.iter().map(|c| Expr {
                    kind: ExprKind::Var(c.snapshot.clone()), pos: e.pos,
                }));
                d.calls += 1;
            }
            Ok(())
        }
        // Ownership wrappers may contain a direct lambda call or a value
        // escape. Traverse their operand before the ordinary ownership checker
        // validates move/borrow/copy semantics.
        ExprKind::Ownership(_, _, x) | ExprKind::Unary(_, x) |
        ExprKind::Trap(x) | ExprKind::Propagate(x) |
        ExprKind::Ascribe(x, _, _) => rewrite_expr(x, d),
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
        ExprKind::Array(args) | ExprKind::New(_, args) => {
            // Constructor arguments are genuine call arguments. Omitting
            // them lets a direct lambda call bypass lexical snapshot rewriting
            // (or silently resolve to a same-named top-level function).
            for a in args { rewrite_expr(a, d)?; }
            Ok(())
        }
        ExprKind::Lambda { .. } => Err(error(e.pos, "E_UNSUPPORTED",
            "nested or escaping lambda needs closure lowering")),
        ExprKind::StateRead(_) => Err(error(e.pos, "E_UNSUPPORTED",
            "protected state projections require a lexical state lease, not direct closure lowering")),
        // Exhaustive AST traversal: introducing a new expression kind must
        // require a compiler review of its lexical binding behaviour.
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) |
        ExprKind::Str(_) | ExprKind::Var(_) | ExprKind::SelfRef |
        ExprKind::Spawn(_) => Ok(()),
    }
}

fn calls_any_named(e: &Expr, names: &BTreeSet<String>, depth: usize) -> R<bool> {
    if depth >= MAX_REWRITE_DEPTH {
        return Err(error(e.pos, "E_NESTING", "lambda call scan limit exceeded"));
    }
    let d = depth + 1;
    match &e.kind {
        ExprKind::Await(inner) => return calls_any_named(inner, names, d),
        ExprKind::SpawnTask { func, args, .. } => {
            if names.contains(func) { return Ok(true); }
            for arg in args {
                if calls_any_named(arg, names, d)? { return Ok(true); }
            }
            return Ok(false);
        }
        ExprKind::Task { .. } | ExprKind::SuspLambda { .. } | ExprKind::Duration(_) => return Ok(false),
        ExprKind::Call(name, args) => {
            if names.contains(name) { return Ok(true); }
            for arg in args {
                if calls_any_named(arg, names, d)? { return Ok(true); }
            }
        }
        ExprKind::Unary(_, a) | ExprKind::Ownership(_, _, a)
        | ExprKind::Trap(a) | ExprKind::Propagate(a)
        | ExprKind::Member(a, _) | ExprKind::Ascribe(a, _, _) => return calls_any_named(a, names, d),
        ExprKind::Binary(_, a, b) => {
            return Ok(calls_any_named(a, names, d)? || calls_any_named(b, names, d)?);
        }
        ExprKind::Ternary(a, b, c) => {
            return Ok(calls_any_named(a, names, d)?
                || calls_any_named(b, names, d)?
                || calls_any_named(c, names, d)?);
        }
        ExprKind::MethodCall(a, _, args) => {
            if calls_any_named(a, names, d)? { return Ok(true); }
            for arg in args {
                if calls_any_named(arg, names, d)? { return Ok(true); }
            }
        }
        ExprKind::Array(args) | ExprKind::New(_, args) => {
            for arg in args {
                if calls_any_named(arg, names, d)? { return Ok(true); }
            }
        }
        // A nested expression lambda or protected state projection is not
        // eligible for independent direct-lambda lowering.
        ExprKind::Lambda { .. } => return Ok(true),
        ExprKind::StateRead(_) |
        ExprKind::SelfRef | ExprKind::Spawn(_) |
        ExprKind::Int(_) | ExprKind::Float(_) |
        ExprKind::Bool(_) | ExprKind::Str(_) => {},
        // A local callable used as a value or intrinsic receiver is also
        // unverified in an independent lambda or isolated handler.
        ExprKind::Var(name) => return Ok(names.contains(name)),
    }
    Ok(false)
}


fn calls_any_named_statements(stmts: &[Stmt], names: &BTreeSet<String>, depth: usize) -> R<bool> {
    if depth >= MAX_REWRITE_DEPTH {
        return Err(error(Pos { line: 1, col: 1 }, "E_NESTING", "lambda statement call scan limit exceeded"));
    }
    for stmt in stmts {
        if calls_any_named_stmt(stmt, names, depth + 1)? { return Ok(true); }
    }
    Ok(false)
}

fn calls_any_named_stmt(s: &Stmt, names: &BTreeSet<String>, depth: usize) -> R<bool> {
    if depth >= MAX_REWRITE_DEPTH {
        return Err(error(stmt_pos(s), "E_NESTING", "lambda statement call scan limit exceeded"));
    }
    let next = depth + 1;
    match s {
        Stmt::Bind { init, .. } => calls_any_named(init, names, next),
        Stmt::Assign { value, .. } => calls_any_named(value, names, next),
        Stmt::FieldAssign { target, value, .. } =>
            Ok(calls_any_named(target, names, next)?
                || calls_any_named(value, names, next)?),
        Stmt::StateWrite { body, .. } | Stmt::Block(body)
        | Stmt::Loop { body, .. } | Stmt::UsingTelemetry { body, .. } => calls_any_named_statements(body, names, next),
        Stmt::Expr(e) | Stmt::Throw(e, _) | Stmt::Raise(e, _)
        | Stmt::Yield(e, _) | Stmt::YieldFrom(e, _) =>
            calls_any_named(e, names, next),
        Stmt::Return(Some(e), _) => calls_any_named(e, names, next),
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => Ok(false),
        Stmt::Try { body, catch, finally, .. } => {
            if calls_any_named_statements(body, names, next)? { return Ok(true); }
            if let Some((_, handler)) = catch {
                if calls_any_named_statements(handler, names, next)? { return Ok(true); }
            }
            if let Some(cleanup) = finally {
                if calls_any_named_statements(cleanup, names, next)? { return Ok(true); }
            }
            Ok(false)
        }
        Stmt::Defer { body, args, .. } | Stmt::Recover { body, args, .. } => {
            for arg in args {
                if calls_any_named(arg, names, next)? { return Ok(true); }
            }
            calls_any_named_statements(body, names, next)
        }
        Stmt::If { arms, otherwise, .. } => {
            for (cond, body) in arms {
                if calls_any_named(cond, names, next)?
                    || calls_any_named_statements(body, names, next)? {
                    return Ok(true);
                }
            }
            if let Some(body) = otherwise {
                return calls_any_named_statements(body, names, next);
            }
            Ok(false)
        }
        Stmt::While { cond, body, .. } =>
            Ok(calls_any_named(cond, names, next)?
                || calls_any_named_statements(body, names, next)?),
        Stmt::For { init, cond, update, body, .. } => {
            if let Some(stmt) = init {
                if calls_any_named_stmt(stmt, names, next)? { return Ok(true); }
            }
            if let Some(expr) = cond {
                if calls_any_named(expr, names, next)? { return Ok(true); }
            }
            if let Some(stmt) = update {
                if calls_any_named_stmt(stmt, names, next)? { return Ok(true); }
            }
            calls_any_named_statements(body, names, next)
        }
        Stmt::ForIn { source, body, .. } =>
            Ok(calls_any_named(source, names, next)?
                || calls_any_named_statements(body, names, next)?),
        Stmt::Match { subject, arms, .. } => {
            if calls_any_named(subject, names, next)? { return Ok(true); }
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    if calls_any_named(guard, names, next)? { return Ok(true); }
                }
                if calls_any_named_statements(&arm.body, names, next)? { return Ok(true); }
            }
            Ok(false)
        }
    }
}

// Collect only simple expression-body free variables. Other lambda bodies
// remain under the existing no-capture/effect checker until scoped capture
// analysis and lifetime lowering are implemented.
fn free_names(e: &Expr, names: &mut BTreeSet<String>, depth: usize) -> R<()> {
    if depth >= MAX_REWRITE_DEPTH {
        return Err(error(e.pos, "E_NESTING", "lambda capture walk limit exceeded"));
    }
    match &e.kind {
        ExprKind::Var(name) => { names.insert(name.clone()); }
        ExprKind::Await(a) => free_names(a, names, depth + 1)?,
        ExprKind::SpawnTask { args, .. } => {
            for arg in args { free_names(arg, names, depth + 1)?; }
        }
        ExprKind::Duration(_) => {}
        ExprKind::Task { .. } | ExprKind::SuspLambda { .. } => return Err(error(e.pos, "E_UNSUPPORTED",
            "a direct lambda cannot capture a task { } body or suspending lambda")),
        ExprKind::Unary(_, a) => free_names(a, names, depth + 1)?,
        ExprKind::Binary(_, a, b) => {
            free_names(a, names, depth + 1)?;
            free_names(b, names, depth + 1)?;
        }
        ExprKind::Ternary(c, a, b) => {
            free_names(c, names, depth + 1)?;
            free_names(a, names, depth + 1)?;
            free_names(b, names, depth + 1)?;
        }
        // Keep this match exhaustive. Newly introduced AST expression forms
        // must be reviewed here before direct closure conversion is enabled;
        // otherwise a missed free-variable use could escape capture analysis.
        ExprKind::Int(_) | ExprKind::Float(_) |
        ExprKind::Bool(_) | ExprKind::Str(_) => {},
        ExprKind::Trap(a) | ExprKind::Propagate(a) | ExprKind::Member(a, _) |
        ExprKind::Ascribe(a, _, _) =>
            free_names(a, names, depth + 1)?,
        ExprKind::Call(_, args) | ExprKind::New(_, args) |
        ExprKind::Array(args) => {
            for arg in args { free_names(arg, names, depth + 1)?; }
        }
        ExprKind::MethodCall(recv, _, args) => {
            free_names(recv, names, depth + 1)?;
            for arg in args { free_names(arg, names, depth + 1)?; }
        }
        // A receiver and a nested lambda are outside the direct-call scalar
        // capture profile. The noncapturing typechecker retains the precise
        // E_CAPTURE/E_UNSUPPORTED failure classification for these forms.
        // Capturing or moving ownership through a generated lambda requires
        // a verified lifetime/effect model. Refuse rather than accidentally
        // skipping an owned free variable during closure conversion.
        ExprKind::Ownership(_, _, _) => return Err(error(e.pos, "E_UNSUPPORTED",
            "explicit ownership inside a direct lambda requires verified capture lifetimes")),
        ExprKind::StateRead(_) => return Err(error(e.pos, "E_UNSUPPORTED",
            "protected state projection inside a direct lambda is not lifetime-verified")),
        ExprKind::SelfRef | ExprKind::Spawn(_) | ExprKind::Lambda { .. } => {},
    }
    Ok(())
}

fn stmt_pos(s: &Stmt) -> Pos {
    match s {
        Stmt::Bind { pos, .. } | Stmt::Assign { pos, .. } |
        Stmt::FieldAssign { pos, .. } | Stmt::StateWrite { pos, .. } | Stmt::If { pos, .. } |
        Stmt::While { pos, .. } | Stmt::Loop { pos, .. } |
        Stmt::For { pos, .. } | Stmt::ForIn { pos, .. } |
        Stmt::Match { pos, .. } | Stmt::Throw(_, pos) |
        Stmt::Raise(_, pos) | Stmt::Try { pos, .. } | Stmt::Defer { pos, .. } | Stmt::Recover { pos, .. } |
        Stmt::UsingTelemetry { pos, .. } |
        Stmt::Yield(_, pos) | Stmt::YieldFrom(_, pos) |
        Stmt::Return(_, pos) | Stmt::Break(pos) |
        Stmt::Continue(pos) => *pos,
        Stmt::Block(body) => body.first().map_or(Pos { line: 1, col: 1 }, stmt_pos),
        Stmt::Expr(e) => e.pos,
    }
}
