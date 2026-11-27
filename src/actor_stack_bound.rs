//! Fail-closed static bound for synchronous actor call chains in the browser
//! compiler. This is AOT admission, NOT resumable actor-owned frame lowering.
//! The generator runtime has a separate bounded continuation-frame ABI.
use crate::check::{TExpr, TProgram, TStmt, TE};
use crate::diag::{Diagnostic, Pos};

/// Conservative bound on statically nested source calls reachable from an
/// actor receive function. A recursion cycle is never admitted by this profile.
pub const MAX_ACTOR_STATIC_CALL_DEPTH: usize = 64;

enum Work<'a> { Stmt(&'a TStmt), Expr(&'a TE) }

fn direct_calls(mut work: Vec<Work<'_>>) -> Vec<usize> {
    let mut calls = Vec::new();
    while let Some(item) = work.pop() {
        match item {
            Work::Stmt(stmt) => match stmt {
                TStmt::Set(_, v) | TStmt::GlobalSet(_, v) | TStmt::Expr(v) |
                TStmt::Throw(v) | TStmt::Raise(v) | TStmt::StateRegister(v) =>
                    work.push(Work::Expr(v)),
                TStmt::FieldSet { obj, value, .. } |
                TStmt::StateFieldWrite { obj, value, .. } |
                TStmt::CollectionStore { array: obj, value, .. } => {
                    work.push(Work::Expr(obj)); work.push(Work::Expr(value));
                }
                TStmt::Return(v) => if let Some(v) = v { work.push(Work::Expr(v)); },
                TStmt::Suspend { op, .. } =>
                    work.extend(crate::check::suspend_operands(op).into_iter().map(Work::Expr)),
                TStmt::Try { body, catch, finally, .. } => {
                    work.extend(body.iter().map(Work::Stmt));
                    if let Some(catch) = catch { work.extend(catch.iter().map(Work::Stmt)); }
                    if let Some(finally) = finally { work.extend(finally.iter().map(Work::Stmt)); }
                }
                TStmt::Defer { args, body, .. } | TStmt::Recover { args, body, .. } => {
                    work.extend(args.iter().map(|(_, v)| Work::Expr(v)));
                    work.extend(body.iter().map(Work::Stmt));
                }
                TStmt::StateCriticalWrite(body) => work.extend(body.iter().map(Work::Stmt)),
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
                TStmt::Break | TStmt::Continue | TStmt::Unreachable |
                TStmt::RecoverNext | TStmt::UnwindFrame { .. } => {}
            },
            Work::Expr(expr) => match &expr.e {
                TExpr::Call(idx, args) => {
                    calls.push(*idx as usize);
                    work.extend(args.iter().map(Work::Expr));
                }
                TExpr::RefinementCheck(v, ..) |
                TExpr::IntToFloat(v) | TExpr::FNeg(v) | TExpr::Neg(v) |
                TExpr::Not(v) | TExpr::AdtPayload(v) | TExpr::FieldGet { obj: v, .. } |
                TExpr::StateFieldRead { obj: v, .. } |
                TExpr::CollectionLoad { array: v, .. } |
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
                TExpr::GlobalGet(_) | TExpr::Alloc(_) | TExpr::CollectionAlloc(_) |
                TExpr::Spawn(_) | TExpr::TraceSpanBegin { .. } | TExpr::TraceSpanEnd { .. } => {}
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

fn bound_root(actor_name: &str, root: usize, graph: &[Vec<usize>]) -> Result<(), Diagnostic> {
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
                format!("actor {} reaches a recursive synchronous call cycle", actor_name))),
            0 => {
                // The direct DFS path is already beyond the AOT profile limit;
                // fail immediately instead of growing the compiler stack.
                if traversal.len() >= MAX_ACTOR_STATIC_CALL_DEPTH {
                    return Err(reject("E_ACTOR_STACK_DEPTH",
                        format!("actor {} exceeds {} source call frames", actor_name,
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
            format!("actor {} exceeds {} source call frames", actor_name,
                    MAX_ACTOR_STATIC_CALL_DEPTH)));
    }
    Ok(())
}

/// Called after typing but before emitting any guest Wasm bytes.
pub fn verify(program: &TProgram) -> Result<(), Diagnostic> {
    if program.actors.is_empty() { return Ok(()); }
    let mut graph: Vec<_> = program.funcs.iter()
        .map(|f| direct_calls(f.body.iter().map(Work::Stmt).collect())).collect();
    let typed_count = graph.len();
    for actor in &program.actors {
        if let Some(root) = actor.receive {
            bound_root(&actor.name, root as usize, &graph)?;
        }
        if !actor.inits.is_empty() {
            // Initialization is a separate actor entry, including actors
            // without receive. Charge one frame for the synthesized entry.
            let calls = direct_calls(actor.inits.iter()
                .map(|(_, value)| Work::Expr(value)).collect());
            // Reject malformed indices before adding the synthetic root.
            // Likewise reject malformed reachable indices into the new slot.
            if calls.iter().any(|&i| i >= typed_count) {
                return Err(reject("E_ACTOR_STACK_CALL", "invalid actor initializer call index"));
            }
            graph.push(calls);
            let result = bound_root(&actor.name, typed_count, &graph);
            graph.pop();
            result?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{TActor, TFunc};
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
            tasks: vec![], task_main: false,
            funcs: bodies.into_iter().map(function).collect(),
            generators: vec![], main: 0, imports: vec![], data: vec![],
            heap: false, globals: vec![], strings: false, guest_eh: false,
            raise_tag: false, catch_depth: false,
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
    #[test] fn accepts_exact_frame_limit() {
        let mut bodies = vec![vec![]; MAX_ACTOR_STATIC_CALL_DEPTH];
        for (i, body) in bodies.iter_mut().enumerate().take(MAX_ACTOR_STATIC_CALL_DEPTH - 1) {
            body.push(call((i + 1) as u32));
        }
        assert!(verify(&program(bodies, 0)).is_ok());
    }
    #[test] fn shared_tail_cannot_hide_overdepth_path() {
        // Visit the long tail directly first, then via one extra helper.
        // The cached tail depth must still be counted in the second path.
        let mut bodies = vec![vec![]; MAX_ACTOR_STATIC_CALL_DEPTH + 1];
        bodies[0] = vec![call(1), call(MAX_ACTOR_STATIC_CALL_DEPTH as u32)];
        for (i, body) in bodies.iter_mut().enumerate().take(MAX_ACTOR_STATIC_CALL_DEPTH - 1).skip(1) {
            body.push(call((i + 1) as u32));
        }
        bodies[MAX_ACTOR_STATIC_CALL_DEPTH].push(call(1));
        assert!(verify(&program(bodies, 0)).is_err());
    }
    #[test] fn rejects_deep_acyclic_chains() {
        let mut bodies = vec![vec![]; MAX_ACTOR_STATIC_CALL_DEPTH + 1];
        for (i, body) in bodies.iter_mut().enumerate().take(MAX_ACTOR_STATIC_CALL_DEPTH) {
            body.push(call((i + 1) as u32));
        }
        assert!(verify(&program(bodies, 0)).is_err());
    }
    #[test] fn ignores_unreachable_recursive_helpers() {
        let p = program(vec![vec![], vec![call(1)]], 0);
        assert!(verify(&p).is_ok());
    }
    #[test] fn initializer_only_actor_rejects_recursive_helper() {
        let mut p = program(vec![vec![call(0)]], 0);
        p.actors[0].receive = None;
        p.actors[0].inits.push((0, TE { e: TExpr::Call(0, vec![]), ty: Type::Void }));
        assert!(verify(&p).is_err());
    }
    #[test] fn initializer_charges_its_entry_frame() {
        let mut bodies = vec![vec![]; MAX_ACTOR_STATIC_CALL_DEPTH];
        for (i, body) in bodies.iter_mut().enumerate().take(MAX_ACTOR_STATIC_CALL_DEPTH - 1) {
            body.push(call((i + 1) as u32));
        }
        let mut p = program(bodies, 0);
        p.actors[0].receive = None;
        p.actors[0].inits.push((0, TE { e: TExpr::Call(1, vec![]), ty: Type::Void }));
        assert!(verify(&p).is_ok());
        p.actors[0].inits[0].1.e = TExpr::Call(0, vec![]);
        assert!(verify(&p).is_err());
    }
    #[test] fn refinement_check_cannot_hide_recursive_call() {
        let wrapped = TStmt::Expr(TE {
            e: TExpr::RefinementCheck(Box::new(TE {
                e: TExpr::Call(0, vec![]), ty: Type::Int,
            }), 0, 100, 0),
            ty: Type::Int,
        });
        assert!(verify(&program(vec![vec![wrapped]], 0)).is_err());
    }
    #[test] fn refinement_check_counts_finite_call() {
        let wrapped = TStmt::Expr(TE {
            e: TExpr::RefinementCheck(Box::new(TE {
                e: TExpr::Call(1, vec![]), ty: Type::Int,
            }), 0, 100, 0),
            ty: Type::Int,
        });
        assert!(verify(&program(vec![vec![wrapped], vec![]], 0)).is_ok());
    }
    #[test] fn rejects_bad_typed_indices() {
        let p = program(vec![vec![call(999)]], 0);
        assert!(verify(&p).is_err());
    }
}