//! Executable *closed scalar* Record lowering through the existing checked
//! class/constructor Wasm ABI. This is NOT an open Record, dynamic Map, native
//! value-struct ABI, or general collection/actor/ownership implementation.
//!
//! Only direct local bindings whose initializer is a nonempty fully typed
//! Record of <= 8 int/bool/float/string literals are admitted. The typed
//! contextual preflight already checked every member before this pass.
//! We intern their sorted (name,type) shapes and emit a private class with
//! fixed eight-byte memory slots. Each initializer is bound in source order
//! BEFORE the class allocation, then passed in canonical slot order.
//! Existing type checking owns all actual field loads, construction, and
//! borrow/actor safety. No user-provided field offset or pointer is trusted.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{
    BindKind, ClassDecl, Constructor, Expr, ExprKind, Field, FieldMode,
    Param, Program, Stmt, Type,
};
use crate::collection_types::{LiteralType, TypedBinding};
use crate::diag::{Diagnostic, Pos};
use crate::record_struct_layout::{FieldType, LayoutInterner};

pub const PRIVATE_PREFIX: &str = "__atto_closed_record_";
const MAX_EXECUTABLE_RECORD_SHAPES: usize = 32;
const MAX_EXECUTABLE_RECORD_FIELDS: usize = 8;

fn diagnostic(pos: Pos, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

fn scalar_type(ty: &LiteralType, pos: Pos) -> Result<Type, Diagnostic> {
    match ty {
        LiteralType::Int => Ok(Type::Int),
        LiteralType::Bool => Ok(Type::Bool),
        LiteralType::Float => Ok(Type::Float),
        LiteralType::String => Ok(Type::Str),
        _ => Err(diagnostic(pos, "E_UNSUPPORTED",
            "executable closed Record fields must be owned int/bool/float/string scalars")),
    }
}

fn default_value(ty: Type, pos: Pos) -> Expr {
    let kind = match ty {
        Type::Int => ExprKind::Int(Some(0)),
        Type::Bool => ExprKind::Bool(false),
        Type::Float => ExprKind::Float(0.0f64.to_bits()),
        Type::Str => ExprKind::Str(String::new()),
        _ => unreachable!("scalar_type checked all supported types"),
    };
    Expr {kind, pos}
}

fn generated_class(name: String, fields: &[FieldType], pos: Pos)
    -> Result<ClassDecl, Diagnostic>
{
    let mut fs = Vec::with_capacity(fields.len());
    let mut params = Vec::with_capacity(fields.len());
    let mut body = Vec::with_capacity(fields.len());
    for (slot, (field, ty)) in fields.iter().enumerate() {
        let ty = scalar_type(ty, pos)?;
        let param_name = format!("{PRIVATE_PREFIX}param_{slot}");
        fs.push(Field {
            name: field.clone(), pos, ty, public: true,
            mode: FieldMode::Constructor, init: default_value(ty, pos),
        });
        params.push(Param {name: param_name.clone(), ty, mutable: false, pos});
        body.push(Stmt::FieldAssign {
            target: Expr {kind: ExprKind::SelfRef, pos},
            field: field.clone(),
            value: Expr {kind: ExprKind::Var(param_name), pos},
            pos,
        });
    }
    Ok(ClassDecl {
        name, pos, public: false, state_container: false, fields: fs,
        ctor: Some(Constructor {pos, params, body}), methods: Vec::new(),
    })
}

// Only literal string projection from an already lowerable, directly named
// local closed Record is rewritten. Computed keys, Maps, open Record values,
// aliases and any non-record receiver retain the explicit Index AST and the
// normal E_UNSUPPORTED checker gate. We deliberately do not rewrite nested
// blocks: their different lexical names/shadowing require an SSA scope proof.
fn rewrite_projection(expr: &mut Expr, record_locals: &BTreeSet<String>, depth: usize) {
    if depth >= 64 { return; }
    match &mut expr.kind {
        ExprKind::Index(receiver, key) => {
            rewrite_projection(receiver, record_locals, depth + 1);
            rewrite_projection(key, record_locals, depth + 1);
            if let (ExprKind::Var(name), ExprKind::Str(field)) =
                (&receiver.kind, &key.kind) {
                if record_locals.contains(name) {
                    expr.kind = ExprKind::Member(receiver.clone(), field.clone());
                }
            }
        }
        ExprKind::Unary(_, inner) | ExprKind::Trap(inner) |
        ExprKind::Propagate(inner) | ExprKind::Ownership(_, _, inner) |
        ExprKind::Member(inner, _) =>
            rewrite_projection(inner, record_locals, depth + 1),
        ExprKind::Binary(_, left, right) => {
            rewrite_projection(left, record_locals, depth + 1);
            rewrite_projection(right, record_locals, depth + 1);
        }
        ExprKind::Ternary(test, yes, no) => {
            rewrite_projection(test, record_locals, depth + 1);
            rewrite_projection(yes, record_locals, depth + 1);
            rewrite_projection(no, record_locals, depth + 1);
        }
        ExprKind::Call(_, args) | ExprKind::New(_, args) |
        ExprKind::Array(args) | ExprKind::Tuple(args) => {
            for arg in args { rewrite_projection(arg, record_locals, depth + 1); }
        }
        ExprKind::MethodCall(recv, _, args) => {
            rewrite_projection(recv, record_locals, depth + 1);
            for arg in args { rewrite_projection(arg, record_locals, depth + 1); }
        }
        // These cannot be introduced by the narrow scalar profile.
        ExprKind::Record(_) | ExprKind::Map(_) | ExprKind::Lambda {..} |
        ExprKind::EmptyBraces | ExprKind::Int(_) | ExprKind::Float(_) |
        ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Var(_) |
        ExprKind::SelfRef | ExprKind::StateRead(_) | ExprKind::Spawn(_) => {}
    }
}
fn rewrite_statement(stmt: &mut Stmt, records: &BTreeSet<String>) {
    match stmt {
        Stmt::Bind {init, ..} | Stmt::Assign {value: init, ..} |
        Stmt::Expr(init) | Stmt::Return(Some(init), _) => {
            rewrite_projection(init, records, 0);
        }
        // Nested statements aren't rewritten with an outer namespace
        // without an independently checked lexical SSA shadow/escape proof.
        _ => {}
    }
}

/// Mutate only explicit direct local Record bindings. All other collection
/// expressions retain pre-existing E_UNSUPPORTED rejection in check().
///
/// The caller must run collection_types::analyze on the ORIGINAL program
/// first, and must reject source tokens spelling PRIVATE_PREFIX. The typed
/// binding inventory is produced by the compiler itself, never user data.
pub fn lower(program: &mut Program, checked: &[TypedBinding])
    -> Result<(), Diagnostic>
{
    let mut interner = LayoutInterner::with_shape_limit(MAX_EXECUTABLE_RECORD_SHAPES);
    let mut source_types = BTreeMap::new();
    // Positions are distinct in the parsed source. This avoids confusing
    // aliases with the same local name in separate lexical scopes.
    for b in checked {
        if let LiteralType::Record(_) = &b.value.ty {
            let p = (b.value.pos.line, b.value.pos.col, b.name.clone());
            if source_types.insert(p, b).is_some() {
                return Err(diagnostic(b.value.pos, "E_SHAPE",
                    "ambiguous compiler record type provenance"));
            }
        }
    }

    let mut classes = Vec::new();
    let mut class_type_indices: Vec<u32> = Vec::new();
    let mut ordinal = 0usize;
    for func in &mut program.funcs {
        let mut body = Vec::with_capacity(func.body.len());
        let mut record_locals = BTreeSet::new();
        for statement in std::mem::take(&mut func.body) {
            let Stmt::Bind {kind, name, ty, init, pos} = statement else {
                let mut other = statement;
                rewrite_statement(&mut other, &record_locals);
                body.push(other);
                continue;
            };
            let ExprKind::Record(fields) = init.kind else {
                let mut ordinary = Stmt::Bind {kind, name: name.clone(), ty, init, pos};
                rewrite_statement(&mut ordinary, &record_locals);
                record_locals.remove(&name); // shadowed non-Record binding
                body.push(ordinary);
                continue;
            };
            if kind == BindKind::Const {
                return Err(diagnostic(pos, "E_UNSUPPORTED",
                    "const Record allocation cannot be a compile-time constant"));
            }
            if fields.is_empty() || fields.len() > MAX_EXECUTABLE_RECORD_FIELDS {
                return Err(diagnostic(pos, "E_UNSUPPORTED",
                    "executable closed Record needs 1..=8 scalar fields"));
            }
            let key = (init.pos.line, init.pos.col, name.clone());
            let binding = source_types.get(&key).ok_or_else(|| diagnostic(
                init.pos, "E_SHAPE", "missing compiler-derived closed Record type"))?;
            let LiteralType::Record(canonical) = &binding.value.ty else {
                return Err(diagnostic(init.pos, "E_SHAPE", "closed Record shape lost"));
            };
            if binding.value.children.len() != fields.len() ||
                binding.value.source_to_record_slot.len() != fields.len() {
                return Err(diagnostic(init.pos, "E_SHAPE", "incomplete record effect inventory"));
            }
            let mut source_fields = Vec::with_capacity(fields.len());
            for ((field, _), child) in fields.iter().zip(&binding.value.children) {
                scalar_type(&child.ty, init.pos)?;
                source_fields.push((field.clone(), child.ty.clone()));
            }
            let planned = interner.closed_record(&source_fields).map_err(|e|
                Diagnostic::new(init.pos, "E_SHAPE", format!("invalid static record layout: {e:?}")))?;
            if planned.source_to_slot != binding.value.source_to_record_slot ||
                interner.shape(planned.shape_id).is_none_or(|shape| shape.fields != *canonical) {
                return Err(diagnostic(init.pos, "E_SHAPE",
                    "compiler record layout differs from typed source inventory"));
            }
            if planned.shape_id == class_type_indices.len() {
                let class_name = format!("{PRIVATE_PREFIX}shape_{}", planned.shape_id);
                let type_idx = program.type_names.len() as u32;
                program.type_names.push((class_name.clone(), pos));
                classes.push(generated_class(class_name,
                    &interner.shape(planned.shape_id).expect("interned shape").fields,
                    pos)?);
                class_type_indices.push(type_idx);
            }
            let class_index = *class_type_indices.get(planned.shape_id)
                .ok_or_else(|| diagnostic(pos, "E_SHAPE", "record shape identity was not interned"))?;
            let class_name = program.type_names[class_index as usize].0.clone();

            // Compile all initializers in original source order. Even when
            // admitted fields become richer, never move effects by sorting.
            let mut args: Vec<Option<Expr>> = vec![None; fields.len()];
            for (i, ((_field, value), dest)) in fields.into_iter()
                .zip(&planned.source_to_slot).enumerate() {
                let temp_name = format!("{PRIVATE_PREFIX}tmp_{}", ordinal);
                ordinal += 1;
                let temp_ty = scalar_type(&source_fields[i].1, value.pos)?;
                let value_pos = value.pos;
                body.push(Stmt::Bind {
                    kind: BindKind::Val, name: temp_name.clone(),
                    ty: Some(temp_ty), init: value, pos: value_pos,
                });
                args[*dest] = Some(Expr {
                    kind: ExprKind::Var(temp_name), pos: value_pos
                });
            }
            let mut checked_args = Vec::with_capacity(args.len());
            for arg in args {
                checked_args.push(arg.ok_or_else(|| diagnostic(
                    pos, "E_SHAPE", "static record slot lacks its source initializer"))?);
            }
            record_locals.insert(name.clone());
            body.push(Stmt::Bind {
                kind, name, ty: Some(Type::Named(class_index)),
                init: Expr {
                    kind: ExprKind::New(class_name, checked_args), pos: init.pos,
                },
                pos,
            });
        }
        func.body = body;
    }
    program.classes.extend(classes);
    Ok(())
}
