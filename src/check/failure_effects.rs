//! Conservatively infer escaping source failure capabilities from typed calls.
//! This is compile-time metadata only: it does not lower raise/panic to Wasm.
//! The currently executable browser profile only supports Error + Throwable.

use super::{TExpr, TE, TStmt, TFunc};
use crate::ast::Program;
use crate::diag::{Diagnostic, Pos};

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

pub(super) fn verify(program: &Program, funcs: &[TFunc]) -> Result<(), Diagnostic> {
    // Explicit annotations describe an upper bound even if the body happens
    // not to exercise it in this compilation. Callers must assume it may.
    let mut effects = vec![0u8; funcs.len()];
    for (i, source) in program.funcs.iter().enumerate() {
        effects[i] = source.failure_effects.mask();
    }
    // Monotone fixed point on the finite 3-bit lattice: handles recursion,
    // mutual recursion and order-independent transitive error propagation.
    for _ in 0..funcs.len().saturating_mul(3).saturating_add(1) {
        let previous = effects.clone();
        for (i, typed) in funcs.iter().enumerate() {
            effects[i] |= stmts(&typed.body, &previous);
        }
        if previous == effects {
            break;
        }
    }
    for (i, source) in program.funcs.iter().enumerate() {
        if !source.failure_effects.explicit {
            continue; // Unannotated functions infer their effects.
        }
        let declared = source.failure_effects.mask();
        let escaped = effects[i] & !declared;
        if escaped != 0 {
            let name = if escaped & THROW != 0 { "throws" }
                else if escaped & RAISE != 0 { "raises" }
                else { "panics" };
            return Err(Diagnostic::new(source.pos, "E_EFFECT_TYPE",
                format!("function '{}' has undeclared escaping {}(Error) effect", source.name, name)));
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
}
