//! Bounded fixed-size Array<int> field storage for the browser Wasm backend.
//! Full production parsing and ordinary type/ownership checking remain
//! mandatory. Internal names contain '$', which cannot occur in source
//! identifiers. No public synthetic type, dynamic index, views, resizes,
//! reference elements, escaping Array handles, or actor transfer is admitted.
use crate::ast::*;
use crate::collection_layout_plan::Storage;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

pub(crate) const INTERNAL: &str = "$fixed_i64_array";
pub(crate) const CELL: &str = "$array_cell_";
const MAX_LEN: usize = 64;
type R<T> = Result<T, Diagnostic>;
fn deny(pos: Pos, why: &str) -> Diagnostic {
    Diagnostic::new(pos, "E_COPY_UNPROVEN", why)
}

pub(crate) fn selected(tokens: &[Token]) -> bool {
    tokens.windows(2).any(|p| matches!((&p[0].tok,&p[1].tok),
        (Tok::Ident(a), Tok::Ident(b)) if a == "define" && b == "class"))
        && tokens.windows(3).any(|p| matches!((&p[0].tok,&p[1].tok,&p[2].tok),
            (Tok::Ident(a), Tok::Dot, Tok::Ident(b)) if a == "Array" && b == "of"))
}

pub(crate) fn literal_int(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(Some(n)) => *n <= i64::MAX as u64,
        ExprKind::Unary(UnOp::Neg, v) => matches!(v.kind, ExprKind::Int(None))
            || matches!(v.kind, ExprKind::Int(Some(n)) if n <= i64::MAX as u64),
        _ => false,
    }
}

fn factory(e: &Expr, len: Option<usize>) -> R<usize> {
    let ExprKind::New(name, values) = &e.kind else {
        return Err(deny(e.pos, "fixed Array field requires fresh Array.of integer literals"));
    };
    if name != INTERNAL || values.is_empty() || values.len() > MAX_LEN
        || len.is_some_and(|n| n != values.len()) || !values.iter().all(literal_int) {
        return Err(deny(e.pos, "fixed Array.of requires 1..64 integer literals of the same static length"));
    }
    Ok(values.len())
}

fn ordinary_name(name: &str, pos: Pos) -> R<()> {
    if name == "Array" || name.contains('$') { Err(deny(pos, "reserved fixed Array storage name")) }
    else { Ok(()) }
}

fn normalize_index(field: &mut String, pos: Pos, len: usize) -> R<()> {
    if let Some(raw) = field.strip_prefix(CELL) {
        let n: i64 = raw.parse().map_err(|_| deny(pos, "invalid fixed index"))?;
        let index = if n < 0 { n + len as i64 } else { n };
        if index < 0 || index >= len as i64 {
            return Err(Diagnostic::new(pos, "E_ARRAY_BOUNDS", "fixed Array index outside logical length"));
        }
        *field = format!("{CELL}{index}");
    } else { ordinary_name(field, pos)?; }
    Ok(())
}
fn named_field(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Member(parent, field)
        if matches!(&parent.kind, ExprKind::Var(_)) && !field.starts_with(CELL))
}

// Walk every admitted expression. Array factories are accepted ONLY at the
// two explicit fresh-storage sites (declaration initializer/replacement).
fn expr(e: &mut Expr, len: usize) -> R<()> {
    let pos = e.pos;
    match &mut e.kind {
        ExprKind::Int(_) | ExprKind::Bool(_) => Ok(()),
        ExprKind::Var(name) => ordinary_name(name, pos),
        ExprKind::Unary(_, a) => expr(a, len),
        ExprKind::Binary(_, a, b) => { expr(a, len)?; expr(b, len) }
        ExprKind::Member(base, field) => {
            if field.starts_with(CELL) && !named_field(base) {
                return Err(deny(pos, "fixed Array index requires a named class field"));
            }
            normalize_index(field, pos, len)?;
            expr(base, len)
        }
        ExprKind::New(name, args) if args.is_empty() => ordinary_name(name, pos),
        ExprKind::Ownership(OwnershipOp::Copy | OwnershipOp::DeepCopy, false, a)
            if matches!(a.kind, ExprKind::Var(_)) => expr(a, len),
        _ => Err(deny(pos, "fixed Array profile admits only straight-line local scalar/owner expressions")),
    }
}
fn stmts(body: &mut [Stmt], len: usize) -> R<()> {
    for stmt in body {
        match stmt {
            Stmt::Bind { name, ty, init, pos, .. } => {
                ordinary_name(name, *pos)?;
                if ty.is_some_and(|t| !matches!(t, Type::Int | Type::Bool | Type::Named(_))) {
                    return Err(deny(*pos, "Array values cannot escape their owned field"));
                }
                expr(init, len)?;
            }
            Stmt::Assign { name, value, pos } => { ordinary_name(name, *pos)?; expr(value, len)?; }
            Stmt::FieldAssign { target, field, value, pos } => {
                if field.starts_with(CELL) && !named_field(target) {
                    return Err(deny(*pos, "fixed Array write requires a named class field"));
                }
                normalize_index(field, *pos, len)?;
                expr(target, len)?;
                if matches!(&value.kind, ExprKind::New(name, _) if name == INTERNAL) {
                    if !matches!(&target.kind, ExprKind::Var(_)) || field.starts_with(CELL) {
                        return Err(deny(*pos, "fresh Array replacement requires an outer root slot"));
                    }
                    factory(value, Some(len))?;
                } else { expr(value, len)?; }
            }
            Stmt::Block(inner) => stmts(inner, len)?,
            Stmt::Return(Some(e), _) => expr(e, len)?,
            _ => return Err(deny(Pos::default(), "fixed Array guest requires straight-line main with lexical blocks")),
        }
    }
    Ok(())
}

pub(crate) fn lower(mut p: Program) -> R<Program> {
    let pos = p.funcs.first().map_or(Pos::default(), |f| f.pos);
    if p.funcs.len() != 1 || p.funcs[0].name != "main" || !p.funcs[0].params.is_empty()
        || p.funcs[0].ret != Type::Int || p.funcs[0].is_generator || p.funcs[0].is_pure
        || p.funcs[0].declared_effects.is_some() || !p.actors.is_empty() || !p.enums.is_empty()
        || p.actor_interface.is_some() || p.classes.is_empty() {
        return Err(deny(pos, "fixed Array guest requires ordinary classes and one int main"));
    }
    if p.array_types.is_empty() || p.array_types.iter().any(|t| t.element != "int" || t.layout != Storage::Aos) {
        return Err(deny(pos, "fixed Array guest supports only Array<int> with contiguous layout"));
    }
    // The first bounded shape admits a single static logical length per
    // program. Values need not agree. Every construction allocates fresh.
    let mut len = None;
    let mut array_fields = 0usize;
    let type_id = u32::try_from(p.type_names.len()).map_err(|_| deny(pos, "type budget"))?;
    for c in &mut p.classes {
        ordinary_name(&c.name, c.pos)?;
        if c.state_container || c.ctor.is_some() || !c.methods.is_empty() {
            return Err(deny(c.pos, "fixed Array roots must be constructor-free and method-free"));
        }
        for field in &mut c.fields {
            ordinary_name(&field.name, field.pos)?;
            let init = field.init.as_ref().ok_or_else(|| deny(field.pos, "field initializer required"))?;
            match field.ty {
                Type::Array(id) if (id as usize) < p.array_types.len() => {
                    if !field.referent_mutable || !matches!(field.binding, FieldBinding::Let | FieldBinding::Val) {
                        return Err(deny(field.pos, "fixed Array fields require explicit mut referent authority"));
                    }
                    len = Some(factory(init, len)?);
                    field.ty = Type::Named(type_id);
                    array_fields += 1;
                }
                Type::Int | Type::Bool if !field.referent_mutable &&
                    (literal_int(init) || matches!(init.kind, ExprKind::Bool(_))) => {}
                _ => return Err(deny(field.pos, "unsupported field in fixed Array guest")),
            }
        }
    }
    if array_fields == 0 { return Err(deny(pos, "no admitted fixed Array fields")); }
    let len = len.ok_or_else(|| deny(pos, "missing Array shape"))?;
    // Reject raw references to intrinsic Array; only the validated field
    // type references introduced above may resolve to the private storage.
    if p.type_names.iter().any(|(n,_)| n == "Array" || n.contains('$')) {
        return Err(deny(pos, "unproved Array type reference"));
    }
    stmts(&mut p.funcs[0].body, len)?;
    p.type_names.push((INTERNAL.to_string(), pos));
    p.array_types.clear();
    p.classes.push(ClassDecl {
        name: INTERNAL.to_string(), pos, public: false, state_container: false,
        fields: (0..len).map(|i| Field {
            name: format!("{CELL}{i}"), pos, ty: Type::Int,
            visibility: FieldVisibility::Public, mode: FieldMode::Constructor,
            binding: FieldBinding::Let, referent_mutable: false,
            init: Some(Expr { kind: ExprKind::Int(Some(0)), pos }),
        }).collect(),
        ctor: None, methods: Vec::new(),
    });
    Ok(p)
}
