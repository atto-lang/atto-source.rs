//! Bounded, literal-only immutable Vector copy-update lowering.
//!
//! This is deliberately not a general persistent HAMT/RRB implementation.
//! In this first slice, a known <=8-scalar literal Vector can be `appended`
//! or `updated` into a new guest-owned Vector. It never mutates its source,
//! duplicates borrowed/mutable references, or materializes an unproven value.
//! Dynamic indices, receivers, captured state and nested elements fail closed.

use std::collections::BTreeMap;
use crate::ast::{Expr, ExprKind, Program, Stmt, Type, UnOp};
use crate::diag::{Diagnostic, Pos};

const MAX_SCALARS: usize = 8;
// Compile-time-only provenance is bounded independently of the 64 KiB source
// limit: copying large scalar strings or thousands of snapshots must not
// amplify compiler memory unboundedly before the Wasm output-size check.
const MAX_KNOWN_VECTORS: usize = 128;
const MAX_COPIED_SCALARS: usize = 512;

fn error(at: Pos, code: &'static str, reason: impl Into<String>) -> Diagnostic {
    Diagnostic::new(at, code, reason)
}

fn static_scalar(value: &Expr) -> bool {
    match &value.kind {
        ExprKind::Int(Some(n)) => *n <= i64::MAX as u64,
        ExprKind::Bool(_) | ExprKind::Float(_) | ExprKind::Str(_) => true,
        ExprKind::Unary(UnOp::Neg, inner) => matches!(&inner.kind,
            ExprKind::Int(Some(n)) if *n <= i64::MAX as u64 + 1) ||
            matches!(&inner.kind, ExprKind::Int(None)),
        ExprKind::Unary(UnOp::Plus, inner) =>
            matches!(&inner.kind, ExprKind::Int(Some(n)) if *n <= i64::MAX as u64),
        _ => false,
    }
}

fn vector_type(names: &[(String, Pos)], ty: Option<Type>) -> bool {
    let Some(Type::Named(id)) = ty else { return false };
    names.get(id as usize).is_some_and(|(name,_)|
        name == "@builtin/Vector" || name.starts_with("@builtin/Vector<"))
}

fn replace(
    original: &[Expr], method: &str, arguments: &[Expr], at: Pos,
) -> Result<Vec<Expr>, Diagnostic> {
    let mut out = original.to_vec();
    match method {
        "appended" => {
            if arguments.len() != 1 {
                return Err(error(at, "E_FACTORY_ARITY",
                    "Vector.appended requires one argument"));
            }
            if !static_scalar(&arguments[0]) {
                return Err(error(arguments[0].pos, "E_UNSUPPORTED",
                    "Vector.appended needs a compile-time scalar value in the bounded guest profile"));
            }
            if out.len() >= MAX_SCALARS {
                return Err(error(at, "E_UNSUPPORTED",
                    "Vector persistent update would exceed eight scalar elements"));
            }
            out.push(arguments[0].clone());
        }
        "updated" => {
            if arguments.len() != 2 {
                return Err(error(at, "E_FACTORY_ARITY",
                    "Vector.updated requires index and value"));
            }
            let index = match &arguments[0].kind {
                ExprKind::Int(Some(index)) if *index <= i64::MAX as u64 => *index as usize,
                ExprKind::Int(_) | ExprKind::Unary(UnOp::Neg, _) =>
                    return Err(error(arguments[0].pos, "E_INDEX_BOUNDS",
                        "Vector.updated index is outside the collection bounds")),
                _ => return Err(error(arguments[0].pos, "E_UNSUPPORTED",
                    "Vector.updated requires a static integer index")),
            };
            if index >= out.len() {
                return Err(error(arguments[0].pos, "E_INDEX_BOUNDS",
                    "Vector.updated index is outside the collection bounds"));
            }
            if !static_scalar(&arguments[1]) {
                return Err(error(arguments[1].pos, "E_UNSUPPORTED",
                    "Vector.updated requires a compile-time scalar value"));
            }
            out[index] = arguments[1].clone();
        }
        _ => return Err(error(at, "E_UNSUPPORTED",
            "this persistent Vector method has no verified guest lowering")),
    }
    Ok(out)
}

/// Rewrite only direct local, statically known, immutable Vector literals.
/// The later collection_types pass rechecks every source and result element
/// type and the fixed_sequence_guest pass emits separate owned guest roots.
pub fn lower(program: &mut Program) -> Result<(), Diagnostic> {
    let names = &program.type_names;
    for function in &mut program.funcs {
        let mut known = BTreeMap::<String, (Type, Vec<Expr>)>::new();
        let mut copied_scalars = 0usize;
        for statement in &mut function.body {
            match statement {
                Stmt::Bind {name,ty,init,..} => {
                    let requested = *ty;
                    if let ExprKind::MethodCall(receiver,method,args) = &init.kind {
                        if let ExprKind::Var(source) = &receiver.kind {
                            if let Some((source_type, originals)) = known.get(source) {
                                let updated = replace(originals, method, args, init.pos)?;
                                copied_scalars = copied_scalars.checked_add(updated.len())
                                    .ok_or_else(|| error(init.pos, "E_SOURCE_LIMIT",
                                        "persistent Vector compile-time copy budget exceeded"))?;
                                if copied_scalars > MAX_COPIED_SCALARS {
                                    return Err(error(init.pos, "E_SOURCE_LIMIT",
                                        "persistent Vector compile-time copy budget exceeded"));
                                }
                                if let Some(explicit) = requested {
                                    if explicit != *source_type {
                                        return Err(error(init.pos, "E_TYPE",
                                            "persistent Vector update cannot change its type identity"));
                                    }
                                } else {
                                    *ty = Some(*source_type);
                                }
                                init.kind = ExprKind::Array(updated);
                            } else if vector_type(names,requested) &&
                                matches!(method.as_str(), "appended" | "updated") {
                                return Err(error(init.pos, "E_UNSUPPORTED",
                                    "persistent Vector updates need a known scalar local owner"));
                            }
                        }
                    }
                    if vector_type(names,*ty) {
                        if let ExprKind::Array(items) = &init.kind {
                            if items.len() <= MAX_SCALARS && items.iter().all(static_scalar) {
                                if !known.contains_key(name) && known.len() >= MAX_KNOWN_VECTORS {
                                    return Err(error(init.pos, "E_SOURCE_LIMIT",
                                        "persistent Vector static provenance budget exceeded"));
                                }
                                known.insert(name.clone(), (ty.expect("vector has type"),items.clone()));
                                continue;
                            }
                        }
                    }
                    known.remove(name);
                }
                // An intervening control flow / mutation can affect the
                // lexical name's provenance. Refuse later static expansion
                // rather than use a potentially stale receiver snapshot.
                Stmt::Assign { .. } | Stmt::Block(_) | Stmt::If { .. } |
                Stmt::While { .. } | Stmt::For { .. } |
                Stmt::ForIn { .. } | Stmt::Loop { .. } |
                Stmt::Match { .. } | Stmt::Try { .. } |
                Stmt::StateWrite { .. } | Stmt::Defer { .. } |
                Stmt::Recover { .. } => known.clear(),
                _ => {}
            }
        }
    }
    Ok(())
}
