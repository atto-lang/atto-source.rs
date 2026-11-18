//! Conservatively infer escaping source failure capabilities from typed calls.
//! This is compile-time metadata only: it does not lower raise/panic to Wasm.
//! The currently executable browser profile only supports Error + Throwable.

use super::{TExpr, TE, TStmt, TFunc};
use crate::ast::Program;
use crate::diag::Diagnostic;

const THROW: u8 = 1;
const RAISE: u8 = 2;
const PANIC: u8 = 4;
const ALL: u8 = THROW | RAISE | PANIC;

fn expr(value: &TE, effects: &[u8]) -> u8 {
    match &value.e {
        TExpr::I64(_) | TExpr::F64(_) | TExpr::Bool(_) |
        TExpr::StrConst(_, _) | TExpr::Local(_) | TExpr::EnumVal(_) |
        TExpr::GlobalGet(_) | TExpr::Alloc(_) | TExpr::Spawn(_) |
        TExpr::PrintStr { .. } => 0,
        TExpr::IntToFloat(v) | TExpr::FNeg(v) | TExpr::Neg(v) |
        TExpr::Not(v) | TExpr::AdtIs { obj: v, .. } |
        TExpr::AdtPayload(v) | TExpr::AdtPropagate { source: v, .. } |
        TExpr::Print { value: v, .. } | TExpr::PrintStrVal { value: v, .. } |
        TExpr::FieldGet { obj: v, .. } | TExpr::PrintObj { obj: v, .. } =>
            expr(v, effects),
        TExpr::Bin(_, a, b) | TExpr::Builtin(_, a, b) =>
            expr(a, effects) | expr(b, effects),
        TExpr::Ternary(c, a, b) =>
            expr(c, effects) | expr(a, effects) | expr(b, effects),
        TExpr::Call(idx, args) =>
            effects.get(*idx as usize).copied().unwrap_or(ALL) |
            args.iter().fold(0, |mask, arg| mask | expr(arg, effects)),
        TExpr::Str(_, a, b) =>
            expr(a, effects) | b.as_ref().map_or(0, |v| expr(v, effects)),
        TExpr::AdtNew { payload, .. } =>
            payload.as_ref().map_or(0, |v| expr(v, effects)),
        TExpr::Trap { value, .. } =>
            // Caller-side trap consumes only an ordinary throw; escalation
            // and fatal panic propagate through trap unchanged.
            expr(value, effects) & !THROW,
        TExpr::Send { target, value } =>
            expr(target, effects) | expr(value, effects),
    }
}

fn stmts(body: &[TStmt], effects: &[u8]) -> u8 {
    body.iter().fold(0, |mask, stmt| mask | match stmt {
        TStmt::Set(_, v) | TStmt::GlobalSet(_, v) |
        TStmt::Expr(v) | TStmt::Return(Some(v)) => expr(v, effects),
        TStmt::Throw(value) => THROW | expr(value, effects),
        TStmt::FieldSet { obj, value, .. } =>
            expr(obj, effects) | expr(value, effects),
        TStmt::ForIn { source, body, .. } =>
            expr(source, effects) | stmts(body, effects),
        TStmt::If(arms, otherwise) =>
            arms.iter().fold(0, |m, (cond, branch)| m |
                expr(cond, effects) | stmts(branch, effects)) |
            otherwise.as_ref().map_or(0, |b| stmts(b, effects)),
        TStmt::Loop { cond, body, update } =>
            cond.as_ref().map_or(0, |v| expr(v, effects)) |
            stmts(body, effects) | stmts(update, effects),
        TStmt::Block(body) => stmts(body, effects),
        TStmt::Return(None) | TStmt::Break | TStmt::Continue |
        TStmt::Unreachable => 0,
    })
}

/// Map source declarations to their exact typed function indices. The
/// compiler may synthesize constructors, class allocations and direct lambdas;
/// those have no explicit source annotations but their typed bodies are still
/// included in the transitive fixed point.
pub(super) fn verify(program: &Program, funcs: &[TFunc]) -> Result<(), Diagnostic> {
    let typed_index: std::collections::BTreeMap<&str, usize> = funcs
        .iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
    let mut bounds: Vec<Option<(&crate::ast::FailureEffects, crate::diag::Pos)>> =
        vec![None; funcs.len()];
    for (i, f) in program.funcs.iter().enumerate() {
        bounds[i] = Some((&f.failure_effects, f.pos));
    }
    for c in &program.classes {
        for m in &c.methods {
            let label = format!("{}.{}", c.name, m.name);
            let i = typed_index.get(label.as_str()).copied().ok_or_else(|| {
                Diagnostic::new(m.pos, "E_EFFECT_TYPE",
                    format!("typed class method '{}' missing its effect-checking body", label))
            })?;
            bounds[i] = Some((&m.failure_effects, m.pos));
        }
    }
    let mut receiver_indices = Vec::<(usize, crate::diag::Pos, &str)>::new();
    for a in &program.actors {
        for m in &a.methods {
            let label = format!("{}.{}", a.name, m.name);
            let i = typed_index.get(label.as_str()).copied().ok_or_else(|| {
                Diagnostic::new(m.pos, "E_EFFECT_TYPE",
                    format!("typed actor method '{}' missing its effect-checking body", label))
            })?;
            bounds[i] = Some((&m.failure_effects, m.pos));
        }
        if let Some(r) = &a.receive {
            let label = format!("{}.receive", a.name);
            let i = typed_index.get(label.as_str()).copied().ok_or_else(|| {
                Diagnostic::new(r.pos, "E_EFFECT_TYPE",
                    format!("typed receive handler '{}' missing its effect-checking body", label))
            })?;
            bounds[i] = Some((&r.failure_effects, r.pos));
            receiver_indices.push((i, r.pos, a.name.as_str()));
        }
    }

    // An explicit upper bound must be honored by every caller even if the
    // current body never invokes the failure capability. An unannotated
    // function/method is inferred from all reachable typed call targets.
    let mut effects = vec![0u8; funcs.len()];
    for (i, bound) in bounds.iter().enumerate() {
        if let Some((declared, _)) = bound {
            effects[i] = declared.mask();
        }
    }
    // Monotone fixed point on the finite three-bit lattice. Recursive,
    // mutually recursive and differently ordered definitions converge.
    for _ in 0..funcs.len().saturating_mul(3).saturating_add(1) {
        let previous = effects.clone();
        for (i, typed) in funcs.iter().enumerate() {
            effects[i] |= stmts(&typed.body, &previous);
        }
        if previous == effects {
            break;
        }
    }
    for (i, bound) in bounds.iter().enumerate() {
        let Some((declared, pos)) = bound else { continue; };
        if !declared.explicit { continue; }
        let escaped = effects[i] & !declared.mask();
        if escaped != 0 {
            let name = if escaped & THROW != 0 { "throws" }
                else if escaped & RAISE != 0 { "raises" }
                else { "panics" };
            return Err(Diagnostic::new(*pos, "E_EFFECT_TYPE",
                format!("function/method '{}' has undeclared escaping {}(Error) effect",
                    funcs[i].name, name)));
        }
    }
    // Type errors must take precedence over runtime-admission errors:
    // otherwise a bad method's declared bound could be hidden behind an
    // E_UNSUPPORTED receive gate.
    // A receive handler may not carry even a potential escaping failure
    // through this runtime: no verified supervisor priority CONTROL path
    // or exception-safe teardown is implemented here.
    for (i, pos, actor) in receiver_indices {
        if effects[i] != 0 {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                format!("actor '{}.receive' has escaping failure effects; supervisor CONTROL/error transport is not implemented for this browser target", actor)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_bits_remain_independent() {
        assert_eq!(THROW | RAISE | PANIC, 7);
        assert_eq!((RAISE | PANIC) & !THROW, RAISE | PANIC);
    }

    #[test]
    fn exhaustive_three_bit_effect_algebra_never_hides_raise_or_panic() {
        // All 8 summaries and all 64 possible pairwise call compositions.
        // This verifies the finite transfer algebra, not whole-program soundness.
        for a in 0..=ALL {
            assert_eq!(a & !THROW, a & (RAISE | PANIC));
            assert_eq!((a & !THROW) & RAISE, a & RAISE);
            assert_eq!((a & !THROW) & PANIC, a & PANIC);
            for b in 0..=ALL {
                let composition = a | b;
                let trapped = composition & !THROW;
                assert_eq!(trapped, (a & !THROW) | (b & !THROW));
                assert_eq!(trapped & RAISE, composition & RAISE);
                assert_eq!(trapped & PANIC, composition & PANIC);
                assert_eq!(composition | a, composition);
                assert_eq!(composition | b, composition);
            }
        }
    }
}
