//! Checked first executable scalar Array.of/Tuple.of slice.
//!
//! Both factory kinds retain distinct types in collection_types::analyze.
//! Only direct local, 1..=8 scalar-element factories are lowerable. We reuse
//! the guest class allocator's owned 8-byte slots, never a host array or a
//! forged raw pointer. Static integer index bounds are proved against the
//! compiler-owned layout before any Wasm is emitted.
//!
//! This is not the final contiguous Array ABI: dynamic indexing, resizing,
//! nested elements, sequence transport, mutable element leases and arbitrary
//! function-expression elements must continue to fail closed.

use std::collections::BTreeMap;

use crate::ast::{
    BindKind, ClassDecl, Constructor, Expr, ExprKind, Field, FieldMode, Param, Program, Stmt, Type,
    UnOp,
};
use crate::collection_types::{LiteralType, TypedBinding};
use crate::diag::{Diagnostic, Pos};

pub const PRIVATE_PREFIX: &str = "__atto_fixed_sequence_";
const MAX_EXECUTABLE_ITEMS: usize = 8;
const MAX_EXECUTABLE_BINDINGS: usize = 32;
const MAX_REWRITE_DEPTH: usize = 64;

fn error(at: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(at, code, message)
}

fn item_field(index: usize) -> String {
    // Deliberately not an Atto identifier, so source '.item_0' cannot expose
    // the checked guest ABI or bypass the bounds-checked index operation.
    format!("\u{1f}index_{index}")
}

fn scalar_type(ty: &LiteralType, at: Pos) -> Result<Type, Diagnostic> {
    match ty {
        LiteralType::Int => Ok(Type::Int),
        LiteralType::Bool => Ok(Type::Bool),
        LiteralType::Float => Ok(Type::Float),
        LiteralType::String => Ok(Type::Str),
        _ => Err(error(at, "E_UNSUPPORTED",
            "executable Tuple.of/Array.of needs owned scalar int/bool/float/string elements")),
    }
}

fn default_value(ty: Type, at: Pos) -> Expr {
    let kind = match ty {
        Type::Int => ExprKind::Int(Some(0)),
        Type::Bool => ExprKind::Bool(false),
        Type::Float => ExprKind::Float(0.0_f64.to_bits()),
        Type::Str => ExprKind::Str(String::new()),
        _ => unreachable!("scalar_type checked the only admitted elements"),
    };
    Expr { pos: at, kind }
}

fn generated_class(name: String, types: &[LiteralType], at: Pos) -> Result<ClassDecl, Diagnostic> {
    let mut fields = Vec::with_capacity(types.len());
    let mut params = Vec::with_capacity(types.len());
    let mut body = Vec::with_capacity(types.len());
    for (index, item) in types.iter().enumerate() {
        let ty = scalar_type(item, at)?;
        let field_name = item_field(index);
        let param_name = format!("{PRIVATE_PREFIX}param_{index}");
        fields.push(Field {
            name: field_name.clone(),
            pos: at,
            ty,
            public: true,
            mode: FieldMode::Constructor,
            init: default_value(ty, at),
        });
        params.push(Param { name: param_name.clone(), ty, mutable: false, pos: at });
        body.push(Stmt::FieldAssign {
            target: Expr { kind: ExprKind::SelfRef, pos: at },
            field: field_name,
            value: Expr { kind: ExprKind::Var(param_name), pos: at },
            pos: at,
        });
    }
    Ok(ClassDecl {
        name,
        pos: at,
        public: false,
        state_container: false,
        fields,
        ctor: Some(Constructor { pos: at, params, body }),
        methods: Vec::new(),
    })
}

/// Only a directly named bound Array/Tuple with a literal nonnegative index
/// acquires a field projection. All computed, negative and non-local accesses
/// retain the original Index node and fail closed in the scalar checker.
fn rewrite_projection(
    expr: &mut Expr,
    locals: &BTreeMap<String, usize>,
    depth: usize,
) -> Result<(), Diagnostic> {
    if depth >= MAX_REWRITE_DEPTH {
        return Err(error(expr.pos, "E_NESTING", "indexed sequence expression exceeds depth budget"));
    }
    match &mut expr.kind {
        ExprKind::Index(receiver, index) => {
            rewrite_projection(receiver, locals, depth + 1)?;
            rewrite_projection(index, locals, depth + 1)?;
            if let ExprKind::Var(name) = &receiver.kind {
                if let Some(&length) = locals.get(name) {
                    let field = match &index.kind {
                        ExprKind::Int(Some(i)) if *i < length as u64 =>
                            item_field(*i as usize),
                        ExprKind::Int(Some(_)) | ExprKind::Int(None) |
                        ExprKind::Unary(UnOp::Neg, _) =>
                            return Err(error(index.pos, "E_INDEX_BOUNDS",
                                "static Array/Tuple index is outside the fixed allocation")),
                        _ => return Err(error(index.pos, "E_UNSUPPORTED",
                            "computed Array/Tuple indexes need a runtime checked indexing ABI")),
                    };
                    expr.kind = ExprKind::Member(receiver.clone(), field);
                }
            }
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Trap(inner)
        | ExprKind::Propagate(inner)
        | ExprKind::Ownership(_, _, inner)
        | ExprKind::Member(inner, _) =>
            rewrite_projection(inner, locals, depth + 1)?,
        ExprKind::Binary(_, lhs, rhs) => {
            rewrite_projection(lhs, locals, depth + 1)?;
            rewrite_projection(rhs, locals, depth + 1)?;
        }
        ExprKind::Ternary(cond, yes, no) => {
            rewrite_projection(cond, locals, depth + 1)?;
            rewrite_projection(yes, locals, depth + 1)?;
            rewrite_projection(no, locals, depth + 1)?;
        }
        ExprKind::Call(_, args) | ExprKind::New(_, args) |
        ExprKind::Array(args) | ExprKind::Tuple(args) => {
            for arg in args { rewrite_projection(arg, locals, depth + 1)?; }
        }
        ExprKind::MethodCall(recv, _, args) => {
            rewrite_projection(recv, locals, depth + 1)?;
            for arg in args { rewrite_projection(arg, locals, depth + 1)?; }
        }
        ExprKind::Record(fields) => {
            for (_, arg) in fields { rewrite_projection(arg, locals, depth + 1)?; }
        }
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                rewrite_projection(key, locals, depth + 1)?;
                rewrite_projection(value, locals, depth + 1)?;
            }
        }
        ExprKind::EmptyBraces | ExprKind::Int(_) | ExprKind::Float(_) |
        ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Var(_) |
        ExprKind::SelfRef | ExprKind::StateRead(_) | ExprKind::Spawn(_) |
        ExprKind::Lambda { .. } => {}
    }
    Ok(())
}

fn rewrite_statement(stmt: &mut Stmt, locals: &BTreeMap<String, usize>) -> Result<(), Diagnostic> {
    match stmt {
        Stmt::Bind { init, .. } | Stmt::Assign { value: init, .. } |
        Stmt::Expr(init) | Stmt::Return(Some(init), _) =>
            rewrite_projection(init, locals, 0),
        // Scope-sensitive indexing inside nested branches must wait for
        // lexical SSA proof; do not let an outer binding leak into a shadow.
        _ => Ok(()),
    }
}

/// Execute a bounded, scalar, factory-owned Array/Tuple in guest Wasm.
///
/// The original source must pass `collection_types::analyze` first. This
/// pass consumes its trusted type inventory and maps each source-evaluated
/// element into a compiler-only class slot. Unsupported sequence values do
/// not receive any guest allocation.
pub fn lower(program: &mut Program, checked: &[TypedBinding]) -> Result<(), Diagnostic> {
    let mut source_types = BTreeMap::new();
    for binding in checked {
        if matches!(&binding.value.ty, LiteralType::Tuple(_) | LiteralType::Array(_)) {
            let key = (binding.value.pos.line, binding.value.pos.col, binding.name.clone());
            if source_types.insert(key, binding).is_some() {
                return Err(error(binding.value.pos, "E_SHAPE",
                    "ambiguous typed sequence source provenance"));
            }
        }
    }

    let mut emitted = Vec::new();
    let mut ordinal = 0usize;
    let mut temp_id = 0usize;
    for func in &mut program.funcs {
        let mut body = Vec::with_capacity(func.body.len());
        let mut locals = BTreeMap::<String, usize>::new();
        for original in std::mem::take(&mut func.body) {
            let Stmt::Bind { kind, name, ty, init, pos } = original else {
                let mut other = original;
                rewrite_statement(&mut other, &locals)?;
                body.push(other);
                continue;
            };
            let raw_items = match &init.kind {
                ExprKind::Tuple(_) => true,
                ExprKind::Call(factory, _) if factory == "@builtin/Array.of" => true,
                _ => false,
            };
            if !raw_items {
                let mut other = Stmt::Bind { kind, name: name.clone(), ty, init, pos };
                rewrite_statement(&mut other, &locals)?;
                locals.remove(&name);
                body.push(other);
                continue;
            }
            let key = (init.pos.line, init.pos.col, name.clone());
            let typed = source_types.get(&key).ok_or_else(|| {
                error(init.pos, "E_SHAPE", "missing compiler-owned Array/Tuple type inventory")
            })?;
            let types = match &typed.value.ty {
                LiteralType::Tuple(items) => items.clone(),
                LiteralType::Array(element) =>
                    vec![element.as_ref().clone(); typed.value.children.len()],
                _ => return Err(error(init.pos, "E_SHAPE", "Array/Tuple type identity lost")),
            };
            if types.is_empty() || types.len() > MAX_EXECUTABLE_ITEMS ||
                ordinal >= MAX_EXECUTABLE_BINDINGS
            {
                return Err(error(init.pos, "E_UNSUPPORTED",
                    "executable Array.of/Tuple.of requires 1..=8 scalar items and <=32 bindings"));
            }
            if kind == BindKind::Const {
                return Err(error(pos, "E_UNSUPPORTED",
                    "const Array/Tuple allocation is not a compile-time constant"));
            }
            if typed.value.children.len() != types.len() {
                return Err(error(init.pos, "E_SHAPE", "typed sequence length differs from source"));
            }
            let values = match init.kind {
                ExprKind::Tuple(items) => items,
                ExprKind::Call(factory, items) if factory == "@builtin/Array.of" => items,
                _ => unreachable!("raw_items checked factory AST"),
            };
            let mut arguments = Vec::with_capacity(values.len());
            for (value, (type_expected, checked_child)) in
                values.into_iter().zip(types.iter().zip(&typed.value.children))
            {
                if &checked_child.ty != type_expected {
                    return Err(error(value.pos, "E_SHAPE",
                        "typed sequence element disagrees with checked factory type"));
                }
                let element_type = scalar_type(type_expected, value.pos)?;
                let var = format!("{PRIVATE_PREFIX}temp_{temp_id}");
                temp_id += 1;
                let value_pos = value.pos;
                let mut initializer = value;
                rewrite_projection(&mut initializer, &locals, 0)?;
                body.push(Stmt::Bind {
                    kind: BindKind::Val,
                    name: var.clone(),
                    ty: Some(element_type),
                    init: initializer,
                    pos: value_pos,
                });
                arguments.push(Expr { kind: ExprKind::Var(var), pos: value_pos });
            }
            let class_name = format!("{PRIVATE_PREFIX}shape_{ordinal}");
            let type_index = program.type_names.len() as u32;
            program.type_names.push((class_name.clone(), init.pos));
            emitted.push(generated_class(class_name.clone(), &types, init.pos)?);
            ordinal += 1;
            locals.insert(name.clone(), types.len());
            body.push(Stmt::Bind {
                kind,
                name,
                ty: Some(Type::Named(type_index)),
                init: Expr { kind: ExprKind::New(class_name, arguments), pos: init.pos },
                pos,
            });
        }
        func.body = body;
    }
    program.classes.extend(emitted);
    Ok(())
}
