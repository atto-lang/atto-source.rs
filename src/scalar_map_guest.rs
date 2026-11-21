//! Bounded immutable Map.of guest allocation and compile-time-proven key lookup.
//!
//! This is NOT the general Map<K,V> runtime. Every key must be a static scalar
//! literal, and only direct local constant-key accesses are lowered. A real
//! guest-owned object stores all key/value pairs, preserving source order and
//! rejecting duplicate constant keys. Dynamic hashes/equality, mutation and
//! aliases remain unsupported rather than silently using a host hash map.
//!
//! The compiler's type-only inventory must be checked before calling lower.
//! The emitted class slots reuse the checked scalar guest object ABI.
//! This pass deliberately enforces a maximum of four entries (eight fields).

use std::collections::BTreeMap;

use crate::ast::{BindKind, Expr, ExprKind, Program, Stmt, Type, UnOp};
use crate::collection_types::{LiteralType, TypedBinding};
use crate::diag::{Diagnostic, Pos};
use crate::fixed_sequence_guest::{generated_class, item_field, scalar_type, PRIVATE_PREFIX};

const MAX_ENTRIES: usize = 4;
const MAX_BINDINGS: usize = 32;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum StaticKey {
    String(String),
    Int(i64),
    Bool(bool),
}

fn error(pos: Pos, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, msg)
}

fn static_key(expr: &Expr) -> Option<StaticKey> {
    match &expr.kind {
        ExprKind::Str(s) => Some(StaticKey::String(s.clone())),
        ExprKind::Bool(v) => Some(StaticKey::Bool(*v)),
        ExprKind::Int(Some(n)) if *n <= i64::MAX as u64 => Some(StaticKey::Int(*n as i64)),
        ExprKind::Unary(UnOp::Neg, inner) => match &inner.kind {
            ExprKind::Int(Some(n)) if *n <= i64::MAX as u64 => Some(StaticKey::Int(-(*n as i64))),
            ExprKind::Int(None) => Some(StaticKey::Int(i64::MIN)),
            _ => None,
        },
        _ => None,
    }
}

fn rewrite_projection(expr: &mut Expr, locals: &BTreeMap<String, BTreeMap<StaticKey, usize>>, depth: usize) -> Result<(), Diagnostic> {
    if depth >= MAX_DEPTH {
        return Err(error(expr.pos, "E_NESTING", "Map.of lookup expression exceeds static depth limit"));
    }
    match &mut expr.kind {
        ExprKind::Index(receiver, key) => {
            rewrite_projection(receiver, locals, depth + 1)?;
            rewrite_projection(key, locals, depth + 1)?;
            if let ExprKind::Var(name) = &receiver.kind {
                if let Some(entries) = locals.get(name) {
                    let id = static_key(key).ok_or_else(|| {
                        error(
                            key.pos,
                            "E_UNSUPPORTED",
                            "Map.of runtime key evaluation/hash lookup is not implemented",
                        )
                    })?;
                    let slot = entries.get(&id).copied().ok_or_else(|| {
                        error(
                            key.pos,
                            "E_UNSUPPORTED",
                            "statically absent Map.of key needs checked missing-key behavior",
                        )
                    })?;
                    expr.kind = ExprKind::Member(receiver.clone(), item_field(slot * 2 + 1));
                }
            }
        }
        ExprKind::Unary(_, v) | ExprKind::Trap(v) | ExprKind::Propagate(v) | ExprKind::Ownership(_, _, v) | ExprKind::Member(v, _) => {
            rewrite_projection(v, locals, depth + 1)?
        }
        ExprKind::Binary(_, l, r) => {
            rewrite_projection(l, locals, depth + 1)?;
            rewrite_projection(r, locals, depth + 1)?;
        }
        ExprKind::Ternary(cond, yes, no) => {
            rewrite_projection(cond, locals, depth + 1)?;
            rewrite_projection(yes, locals, depth + 1)?;
            rewrite_projection(no, locals, depth + 1)?;
        }
        ExprKind::Call(_, args) | ExprKind::New(_, args) | ExprKind::Array(args) | ExprKind::Tuple(args) => {
            for arg in args {
                rewrite_projection(arg, locals, depth + 1)?;
            }
        }
        ExprKind::MethodCall(receiver, _, args) => {
            rewrite_projection(receiver, locals, depth + 1)?;
            for arg in args {
                rewrite_projection(arg, locals, depth + 1)?;
            }
        }
        ExprKind::Record(fields) => {
            for (_, value) in fields {
                rewrite_projection(value, locals, depth + 1)?;
            }
        }
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                rewrite_projection(key, locals, depth + 1)?;
                rewrite_projection(value, locals, depth + 1)?;
            }
        }
        ExprKind::EmptyBraces
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Symbol(_)
        | ExprKind::Entry(_, _)
        | ExprKind::Var(_)
        | ExprKind::SelfRef
        | ExprKind::StateRead(_)
        | ExprKind::Spawn(_)
        | ExprKind::Lambda { .. } => {}
    }
    Ok(())
}

fn rewrite_statement(stmt: &mut Stmt, locals: &BTreeMap<String, BTreeMap<StaticKey, usize>>) -> Result<(), Diagnostic> {
    match stmt {
        Stmt::Bind { init, .. } | Stmt::Assign { value: init, .. } | Stmt::Expr(init) | Stmt::Return(Some(init), _) => {
            rewrite_projection(init, locals, 0)
        }
        // No lexical SSA proof for nested binders yet. The ordinary checker
        // must reject the unlowered Map operations in these scopes.
        _ => Ok(()),
    }
}

// The frozen const factory admits only static scalar expressions. The
// executable object is an owned compiler-private immutable guest aggregate;
// this is not authority to evaluate arbitrary host effects at compile time.
fn constant_scalar(value: &Expr) -> bool {
    match &value.kind {
        ExprKind::Int(Some(_)) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) => true,
        ExprKind::Unary(UnOp::Neg | UnOp::Plus, inner) => matches!(inner.kind, ExprKind::Int(Some(_)) | ExprKind::Float(_)),
        _ => false,
    }
}

fn add_scalar(body: &mut Vec<Stmt>, args: &mut Vec<Expr>, value: Expr, ty: Type, temp_id: &mut usize) {
    let pos = value.pos;
    let name = format!("{PRIVATE_PREFIX}map_tmp_{}", *temp_id);
    *temp_id += 1;
    body.push(Stmt::Bind {
        kind: BindKind::Val,
        name: name.clone(),
        ty: Some(ty),
        init: value,
        pos,
    });
    args.push(Expr {
        kind: ExprKind::Var(name),
        pos,
    });
}

/// Lower direct, statically keyed scalar Map.of bindings into real owned
/// guest objects. No dynamic map operations are invented by this pass.
pub fn lower(program: &mut Program, checked: &[TypedBinding]) -> Result<(), Diagnostic> {
    let mut inventory = BTreeMap::new();
    for binding in checked {
        if matches!(&binding.value.ty, LiteralType::Map(_, _)) {
            let key = (binding.value.pos.line, binding.value.pos.col, binding.name.clone());
            if inventory.insert(key, binding).is_some() {
                return Err(error(binding.value.pos, "E_SHAPE", "ambiguous checked Map.of type provenance"));
            }
        }
    }
    let mut classes = Vec::new();
    let mut ordinal = 0usize;
    let mut temp_id = 0usize;
    for func in &mut program.funcs {
        let mut locals = BTreeMap::<String, BTreeMap<StaticKey, usize>>::new();
        let mut body = Vec::with_capacity(func.body.len());
        for source in std::mem::take(&mut func.body) {
            let Stmt::Bind { kind, name, ty, init, pos } = source else {
                let mut ordinary = source;
                rewrite_statement(&mut ordinary, &locals)?;
                body.push(ordinary);
                continue;
            };
            let explicit_entries = matches!(&init.kind,
                ExprKind::Call(name, _) if name.starts_with("@builtin/Map.of_entries<"));
            let parsed_map = match &init.kind {
                ExprKind::Map(entries) => Some(entries.clone()),
                ExprKind::Call(factory, args) if factory.starts_with("@builtin/Map.of_entries<") => {
                    if args.len() % 2 != 0 {
                        return Err(error(
                            init.pos,
                            "E_FACTORY_ARITY",
                            "Map.of_entries key/value pair count is inconsistent",
                        ));
                    }
                    Some(args.chunks_exact(2).map(|entry| (entry[0].clone(), entry[1].clone())).collect())
                }
                ExprKind::EmptyBraces => {
                    // {} can become a Map only when the checked type
                    // inventory supplies a concrete Map<K,V> context.
                    let key = (init.pos.line, init.pos.col, name.clone());
                    if inventory.get(&key).is_some_and(|b| matches!(&b.value.ty, LiteralType::Map(_, _))) {
                        Some(Vec::new())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some(entries) = parsed_map else {
                let mut ordinary = Stmt::Bind {
                    kind,
                    name: name.clone(),
                    ty,
                    init,
                    pos,
                };
                rewrite_statement(&mut ordinary, &locals)?;
                locals.remove(&name);
                body.push(ordinary);
                continue;
            };
            if kind == BindKind::Const {
                if !explicit_entries {
                    return Err(error(
                        pos,
                        "E_UNSUPPORTED",
                        "const Map.of allocation requires an explicit statically typed Map.of_entries factory",
                    ));
                }
                if !entries
                    .iter()
                    .all(|(key, value)| static_key(key).is_some() && constant_scalar(value))
                {
                    return Err(error(
                        pos,
                        "E_CONST",
                        "const Map.of_entries requires only compile-time scalar key/value expressions",
                    ));
                }
            }
            if entries.len() > MAX_ENTRIES || ordinal >= MAX_BINDINGS {
                return Err(error(
                    pos,
                    "E_UNSUPPORTED",
                    "bounded Map.of guest allocation permits 0..=4 typed scalar pairs and <=32 maps",
                ));
            }
            let at = (init.pos.line, init.pos.col, name.clone());
            let binding = inventory
                .get(&at)
                .ok_or_else(|| error(init.pos, "E_SHAPE", "Map.of is missing compiler-owned static key/value inventory"))?;
            let LiteralType::Map(key_type, value_type) = &binding.value.ty else {
                return Err(error(init.pos, "E_SHAPE", "typed Map identity was lost"));
            };
            // Untyped Map.of() is not a concrete Map<K,V>. Reject instead
            // of creating a type-erased or host-backed dictionary.
            if entries.is_empty() && matches!(key_type.as_ref(), LiteralType::Bottom) {
                return Err(error(
                    init.pos,
                    "E_TYPE_INFERENCE",
                    "empty Map needs an explicit concrete Map<K,V> type",
                ));
            }
            if !matches!(key_type.as_ref(), LiteralType::Int | LiteralType::Bool | LiteralType::String) {
                return Err(error(
                    init.pos,
                    "E_MAP_KEY",
                    "executable Map.of requires homogeneous int/bool/string literal keys",
                ));
            }
            let key_type_scalar = scalar_type(key_type, init.pos)?;
            let value_type_scalar = scalar_type(value_type, init.pos)?;
            if binding.value.children.len() != entries.len() * 2 {
                return Err(error(
                    init.pos,
                    "E_SHAPE",
                    "Map.of key/value inventory length differs from parsed source",
                ));
            }
            let mut lookup = BTreeMap::<StaticKey, usize>::new();
            let mut types = Vec::with_capacity(entries.len() * 2);
            let mut args = Vec::with_capacity(entries.len() * 2);
            for (i, (key, value)) in entries.into_iter().enumerate() {
                let k = static_key(&key).ok_or_else(|| {
                    error(
                        key.pos,
                        "E_UNSUPPORTED",
                        "Map.of executable keys must be compile-time scalar literals",
                    )
                })?;
                if lookup.insert(k, i).is_some() {
                    return Err(error(key.pos, "E_DUPLICATE", "Map.of keys collide under canonical scalar equality"));
                }
                if binding.value.children[2 * i].ty != **key_type || binding.value.children[2 * i + 1].ty != **value_type {
                    return Err(error(
                        value.pos,
                        "E_SHAPE",
                        "Map.of source key/value type differs from checked inventory",
                    ));
                }
                types.push(key_type.as_ref().clone());
                types.push(value_type.as_ref().clone());
                add_scalar(&mut body, &mut args, key, key_type_scalar, &mut temp_id);
                add_scalar(&mut body, &mut args, value, value_type_scalar, &mut temp_id);
            }
            let class_name = format!("{PRIVATE_PREFIX}map_shape_{ordinal}");
            let class_type = program.type_names.len() as u32;
            program.type_names.push((class_name.clone(), init.pos));
            // One compiler-private int(0) sentinel occupies guest
            // allocation for an empty Map; there is no runtime key slot.
            // In particular, never call the zero-size bump allocator.
            let cls = if types.is_empty() {
                args.push(Expr {
                    kind: ExprKind::Int(Some(0)),
                    pos: init.pos,
                });
                generated_class(class_name.clone(), &[LiteralType::Int], init.pos)?
            } else {
                generated_class(class_name.clone(), &types, init.pos)?
            };
            classes.push(cls);
            ordinal += 1;
            locals.insert(name.clone(), lookup);
            body.push(Stmt::Bind {
                // A const Map.of_entries is statically evaluated and frozen
                // by construction. The checked Wasm ABI allocates an owned
                // immutable guest handle; this internal Val cannot mutate it.
                kind: if kind == BindKind::Const { BindKind::Val } else { kind },
                name,
                ty: Some(Type::Named(class_type)),
                init: Expr {
                    kind: ExprKind::New(class_name, args),
                    pos: init.pos,
                },
                pos,
            });
        }
        func.body = body;
    }
    program.classes.extend(classes);
    Ok(())
}
