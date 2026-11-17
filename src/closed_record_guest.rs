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

use std::collections::BTreeMap;

use crate::ast::{
    BindKind, ClassDecl, Constructor, Expr, ExprKind, Field, FieldMode, Param, Program, Stmt, Type,
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
        _ => Err(diagnostic(
            pos,
            "E_UNSUPPORTED",
            "executable closed Record fields must be owned int/bool/float/string scalars",
        )),
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
    Expr { kind, pos }
}

fn generated_class(name: String, fields: &[FieldType], pos: Pos) -> Result<ClassDecl, Diagnostic> {
    let mut fs = Vec::with_capacity(fields.len());
    let mut params = Vec::with_capacity(fields.len());
    let mut body = Vec::with_capacity(fields.len());
    for (slot, (field, ty)) in fields.iter().enumerate() {
        let ty = scalar_type(ty, pos)?;
        let param_name = format!("{PRIVATE_PREFIX}param_{slot}");
        fs.push(Field {
            name: field.clone(),
            pos,
            ty,
            public: true,
            mode: FieldMode::Constructor,
            init: default_value(ty, pos),
        });
        params.push(Param {
            name: param_name.clone(),
            ty,
            mutable: false,
            pos,
        });
        body.push(Stmt::FieldAssign {
            target: Expr {
                kind: ExprKind::SelfRef,
                pos,
            },
            field: field.clone(),
            value: Expr {
                kind: ExprKind::Var(param_name),
                pos,
            },
            pos,
        });
    }
    Ok(ClassDecl {
        name,
        pos,
        public: false,
        state_container: false,
        fields: fs,
        ctor: Some(Constructor { pos, params, body }),
        methods: Vec::new(),
    })
}

// The source Index AST is intentionally left intact. The typed checker
// validates the resolved receiver's actual class identity and a literal
// string field name at the point of use, including lexical shadowing and
// nested blocks. No identifier-based AST rewrite can grant field access.
//
// Only direct local *construction* is supported in this initial profile.
/// Mutate only explicit direct local Record bindings. All other collection
/// expressions retain pre-existing E_UNSUPPORTED rejection in check().
///
/// The caller must run collection_types::analyze on the ORIGINAL program
/// first, and must reject source tokens spelling PRIVATE_PREFIX. The typed
/// binding inventory is produced by the compiler itself, never user data.
pub fn lower(program: &mut Program, checked: &[TypedBinding]) -> Result<(), Diagnostic> {
    let mut interner = LayoutInterner::with_shape_limit(MAX_EXECUTABLE_RECORD_SHAPES);
    let mut source_types = BTreeMap::new();
    // Positions are distinct in the parsed source. This avoids confusing
    // aliases with the same local name in separate lexical scopes.
    for b in checked {
        if let LiteralType::Record(_) = &b.value.ty {
            let p = (b.value.pos.line, b.value.pos.col, b.name.clone());
            if source_types.insert(p, b).is_some() {
                return Err(diagnostic(
                    b.value.pos,
                    "E_SHAPE",
                    "ambiguous compiler record type provenance",
                ));
            }
        }
    }

    let mut classes = Vec::new();
    let mut class_type_indices: Vec<u32> = Vec::new();
    let mut ordinal = 0usize;
    for func in &mut program.funcs {
        let mut body = Vec::with_capacity(func.body.len());
        for statement in std::mem::take(&mut func.body) {
            let Stmt::Bind {
                kind,
                name,
                ty,
                init,
                pos,
            } = statement
            else {
                body.push(statement);
                continue;
            };
            let ExprKind::Record(fields) = init.kind else {
                body.push(Stmt::Bind {
                    kind,
                    name,
                    ty,
                    init,
                    pos,
                });
                continue;
            };
            if kind == BindKind::Const {
                return Err(diagnostic(
                    pos,
                    "E_UNSUPPORTED",
                    "const Record allocation cannot be a compile-time constant",
                ));
            }
            if fields.is_empty() || fields.len() > MAX_EXECUTABLE_RECORD_FIELDS {
                return Err(diagnostic(
                    pos,
                    "E_UNSUPPORTED",
                    "executable closed Record needs 1..=8 scalar fields",
                ));
            }
            let key = (init.pos.line, init.pos.col, name.clone());
            let binding = source_types.get(&key).ok_or_else(|| {
                diagnostic(
                    init.pos,
                    "E_SHAPE",
                    "missing compiler-derived closed Record type",
                )
            })?;
            let LiteralType::Record(canonical) = &binding.value.ty else {
                return Err(diagnostic(init.pos, "E_SHAPE", "closed Record shape lost"));
            };
            if binding.value.children.len() != fields.len()
                || binding.value.source_to_record_slot.len() != fields.len()
            {
                return Err(diagnostic(
                    init.pos,
                    "E_SHAPE",
                    "incomplete record effect inventory",
                ));
            }
            let mut source_fields = Vec::with_capacity(fields.len());
            for ((field, _), child) in fields.iter().zip(&binding.value.children) {
                scalar_type(&child.ty, init.pos)?;
                source_fields.push((field.clone(), child.ty.clone()));
            }
            let planned = interner.closed_record(&source_fields).map_err(|e| {
                Diagnostic::new(
                    init.pos,
                    "E_SHAPE",
                    format!("invalid static record layout: {e:?}"),
                )
            })?;
            if planned.source_to_slot != binding.value.source_to_record_slot
                || interner
                    .shape(planned.shape_id)
                    .is_none_or(|shape| shape.fields != *canonical)
            {
                return Err(diagnostic(
                    init.pos,
                    "E_SHAPE",
                    "compiler record layout differs from typed source inventory",
                ));
            }
            if planned.shape_id == class_type_indices.len() {
                let class_name = format!("{PRIVATE_PREFIX}shape_{}", planned.shape_id);
                let type_idx = program.type_names.len() as u32;
                program.type_names.push((class_name.clone(), pos));
                classes.push(generated_class(
                    class_name,
                    &interner
                        .shape(planned.shape_id)
                        .expect("interned shape")
                        .fields,
                    pos,
                )?);
                class_type_indices.push(type_idx);
            }
            let class_index = *class_type_indices.get(planned.shape_id).ok_or_else(|| {
                diagnostic(pos, "E_SHAPE", "record shape identity was not interned")
            })?;
            let class_name = program.type_names[class_index as usize].0.clone();

            // Compile all initializers in original source order. Even when
            // admitted fields become richer, never move effects by sorting.
            let mut args: Vec<Option<Expr>> = vec![None; fields.len()];
            for (i, ((_field, value), dest)) in
                fields.into_iter().zip(&planned.source_to_slot).enumerate()
            {
                let temp_name = format!("{PRIVATE_PREFIX}tmp_{}", ordinal);
                ordinal += 1;
                let temp_ty = scalar_type(&source_fields[i].1, value.pos)?;
                let value_pos = value.pos;
                body.push(Stmt::Bind {
                    kind: BindKind::Val,
                    name: temp_name.clone(),
                    ty: Some(temp_ty),
                    init: value,
                    pos: value_pos,
                });
                args[*dest] = Some(Expr {
                    kind: ExprKind::Var(temp_name),
                    pos: value_pos,
                });
            }
            let mut checked_args = Vec::with_capacity(args.len());
            for arg in args {
                checked_args.push(arg.ok_or_else(|| {
                    diagnostic(
                        pos,
                        "E_SHAPE",
                        "static record slot lacks its source initializer",
                    )
                })?);
            }
            body.push(Stmt::Bind {
                kind,
                name,
                ty: Some(Type::Named(class_index)),
                init: Expr {
                    kind: ExprKind::New(class_name, checked_args),
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
