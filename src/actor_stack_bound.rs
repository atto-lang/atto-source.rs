//! Fail-closed static bound for synchronous actor call chains in the browser
//! compiler. This is AOT admission, NOT resumable actor-owned frame lowering.
//! The generator runtime has a separate bounded continuation-frame ABI.
use crate::check::{TActor, TExpr, TProgram, TStmt, TE};
use crate::diag::{Diagnostic, Pos};

/// Conservative bound on statically nested source calls reachable from an
/// actor receive function. A recursion cycle is never admitted by this profile.
pub const MAX_ACTOR_STATIC_CALL_DEPTH: usize = 64;

enum Work<'a> { Stmt(&'a TStmt), Expr(&'a TE) }

fn direct_calls(stmts: &[TStmt]) -> Vec<usize> {
    let mut work: Vec<Work<'_>> = stmts.iter().map(Work::Stmt).collect();
    let mut calls = Vec::new();
    while let Some(item) = work.pop() {
        match item {
            Work::Stmt(stmt) => match stmt {
                TStmt::Set(_, v) | TStmt::GlobalSet(_, v) | TStmt::Expr(v) |
                TStmt::Throw(v) => work.push(Work::Expr(v)),
                TStmt::FieldSet { obj, value, .. } => {
                    work.push(Work::Expr(obj)); work.push(Work::Expr(value));
                }
                TStmt::Return(v) => if let Some(v) = v { work.push(Work::Expr(v)); },
                TStmt::ForIn { source, body, .. } => {
                    work.push(Work::Expr(source));
                    work.extend(body.iter().map(Work::Stmt));
                }
                TStmt::If(arms, otherwise) => {
                    for (condition, body) in arms {
                        work.push(Work::Expr(condition));
                        work.extend(body.iter().map(Work::Stmt));
                    }
                    if let Some(body) = otherwise { work.extend(body.iter().map(Work::Stmt)); }
                }
                TStmt::Loop { cond, body, update } => {
                    if let Some(v) = cond { work.push(Work::Expr(v)); }
                    work.extend(body.iter().map(Work::Stmt));
                    work.extend(update.iter().map(Work::Stmt));
                }
                TStmt::Block(body) => work.extend(body.iter().map(Work::Stmt)),
                TStmt::Break | TStmt::Continue | TStmt::Unreachable => {}
            },
            Work::Expr(expr) => match &expr.e {
                TExpr::Call(idx, args) => {
                    calls.push(*idx as usize);
                    work.extend(args.iter().map(Work::Expr));
                }
                TExpr::IntToFloat(v) | TExpr::FNeg(v) | TExpr::Neg(v) |
                TExpr::Not(v) | TExpr::AdtPayload(v) | TExpr::FieldGet { obj: v, .. } |
                TExpr::AdtIs { obj: v, .. } | TExpr::AdtPropagate { source: v, .. } |
                TExpr::Print { value: v, .. } | TExpr::PrintStrVal { value: v, .. } |
                TExpr::Trap { value: v, .. } | TExpr::PrintObj { obj: v, .. } => {
                    work.push(Work::Expr(v));
                }
                TExpr::Bin(_, a, b) | TExpr::Builtin(_, a, b) => {
                    work.push(Work::Expr(a)); work.push(Work::Expr(b));
                }
                TExpr::Ternary(a, b, c) => {
                    work.push(Work::Expr(a)); work.push(Work::Expr(b));
                    work.push(Work::Expr(c));
                }
                TExpr::Str(_, v, other) => {
                    work.push(Work::Expr(v));
                    if let Some(other) = other { work.push(Work::Expr(other)); }
                }
                TExpr::Send { target, value } => {
                    work.push(Work::Expr(target)); work.push(Work::Expr(value));
                }
                TExpr::AdtNew { payload, .. } => {
                    if let Some(v) = payload { work.push(Work::Expr(v)); }
                }
                TExpr::I64(_) | TExpr::F64(_) | TExpr::Bool(_) | TExpr::StrConst(..) |
                TExpr::Local(_) | TExpr::PrintStr { .. } | TExpr::EnumVal(_) |
                TExpr::GlobalGet(_) | TExpr::Alloc(_) | TExpr::Spawn(_) => {}
            }
        }
    }
    calls.sort_unstable();
    calls.dedup();
    calls
}

fn reject(code: &'static str, detail: impl Into<String>) -> Diagnostic {
    // Typed IR currently has no call-site span; future MIR must retain one.
    Diagnostic::new(Pos { line: 1, col: 1 }, code, detail)
}

fn bound_one_actor(actor: &TActor, graph: &[Vec<usize>]) -> Result<(), Diagnostic> {
    let Some(root) = actor.receive.map(|i| i as usize) else { return Ok(()); };
    if root >= graph.len() {
        return Err(reject("E_ACTOR_STACK_CALL", "actor handler outside typed function table"));
    }
    let mut state = vec![0u8; graph.len()]; // 0 unseen, 1 active, 2 complete
    let mut depth = vec![0usize; graph.len()];
    let mut traversal = vec![(root, 0usize)];
    state[root] = 1;
    while let Some(&(function, next)) = traversal.last() {
        if next == graph[function].len() {
            depth[function] = 1 + graph[function].iter().map(|&i| depth[i]).max().unwrap_or(0);
            state[function] = 2;
            traversal.pop();
            continue;
        }
        traversal.last_mut().unwrap().1 += 1;
        let callee = graph[function][next];
        if callee >= graph.len() {
            return Err(reject("E_ACTOR_STACK_CALL", "invalid typed call index"));
        }
        match state[callee] {
            1 => return Err(reject("E_ACTOR_STACK_RECURSION",
                format!("actor {} reaches a recursive synchronous call cycle", actor.name))),
            0 => {
                // The direct DFS path is already beyond the AOT profile limit;
                // fail immediately instead of growing the compiler stack.
                if traversal.len() >= MAX_ACTOR_STATIC_CALL_DEPTH {
                    return Err(reject("E_ACTOR_STACK_DEPTH",
                        format!("actor {} exceeds {} source call frames", actor.name,
                                MAX_ACTOR_STATIC_CALL_DEPTH)));
                }
                state[callee] = 1;
                traversal.push((callee, 0));
            }
            _ => {}
        }
    }
    if depth[root] > MAX_ACTOR_STATIC_CALL_DEPTH {
        return Err(reject("E_ACTOR_STACK_DEPTH",
            format!("actor {} exceeds {} source call frames", actor.name,
                    MAX_ACTOR_STATIC_CALL_DEPTH)));
    }
    Ok(())
}

/// Called after typing but before emitting any guest Wasm bytes.
pub fn verify(program: &TProgram) -> Result<(), Diagnostic> {
    if program.actors.is_empty() { return Ok(()); }
    let graph: Vec<_> = program.funcs.iter().map(|f| direct_calls(&f.body)).collect();
    for actor in &program.actors { bound_one_actor(actor, &graph)?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::TFunc;
    use crate::ast::Type;
    fn call(idx: u32) -> TStmt {
        TStmt::Expr(TE { e: TExpr::Call(idx, vec![]), ty: Type::Void })
    }
    fn function(body: Vec<TStmt>) -> TFunc {
        TFunc { name: "f".into(), params: vec![], ret: Type::Void,
                locals: vec![], body }
    }
    fn program(bodies: Vec<Vec<TStmt>>, root: u32) -> TProgram {
        TProgram {
            funcs: bodies.into_iter().map(function).collect(),
            generators: vec![], main: 0, imports: vec![], data: vec![],
            heap: false, globals: vec![], strings: false, guest_eh: false,
            actors: vec![TActor { name: "Worker".into(), inits: vec![],
                                 receive: Some(root) }],
        }
    }
    #[test] fn accepts_finite_calls() {
        let p = program(vec![vec![call(1)], vec![call(2)], vec![]], 0);
        assert!(verify(&p).is_ok());
    }
    #[test] fn rejects_direct_recursion() {
        let p = program(vec![vec![call(0)]], 0);
        assert!(verify(&p).is_err());
    }
    #[test] fn rejects_indirect_recursion() {
        let p = program(vec![vec![call(1)], vec![call(0)]], 0);
        assert!(verify(&p).is_err());
    }
    #[test] fn rejects_deep_acyclic_chains() {
        let mut bodies = vec![vec![]; MAX_ACTOR_STATIC_CALL_DEPTH + 1];
        for i in 0..MAX_ACTOR_STATIC_CALL_DEPTH {
            bodies[i].push(call((i + 1) as u32));
        }
        assert!(verify(&program(bodies, 0)).is_err());
    }
    #[test] fn ignores_unreachable_recursive_helpers() {
        let p = program(vec![vec![], vec![call(1)]], 0);
        assert!(verify(&p).is_ok());
    }
    #[test] fn rejects_bad_typed_indices() {
        let p = program(vec![vec![call(999)]], 0);
        assert!(verify(&p).is_err());
    }
}