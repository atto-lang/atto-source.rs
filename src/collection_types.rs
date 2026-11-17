//! Contextual typing for inline collection values (parser/semantic admission only).
//!
//! This module deliberately does NOT allocate, lower or emit guest-Wasm values.
//! Its output is an owned, structurally typed IR, separate from the executable
//! scalar checker. Unsupported expressions fail closed rather than being given
//! an invented runtime type. Once this is wired to checked guest allocation the
//! same type and tree structure can be consumed by the lowering stage.

use crate::ast::{Expr, ExprKind, Program, Stmt, Type, UnOp};
use crate::diag::{Diagnostic, Pos};

pub const MAX_LITERAL_DEPTH: usize = 128;

/// Distinct type constructors, not string aliases for guest heap pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralType {
    Int,
    Bool,
    Float,
    String,
    /// A polymorphic element slot in an empty collection; never a dynamic Any.
    Bottom,
    /// The built-in open Record supertype. Access requires a shape/presence guard.
    OpenRecord,
    /// Preserved concrete field-name -> field-type structure.
    Record(Vec<(String, LiteralType)>),
    Map(Box<LiteralType>, Box<LiteralType>),
    List(Box<LiteralType>),
    Array(Box<LiteralType>),
    /// Unparameterized Tuple accepts any arity; not the same as ().
    AnyTuple,
    /// Concrete element-by-position tuple. Empty = unit tuple.
    Tuple(Vec<LiteralType>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedLiteral {
    pub pos: Pos,
    pub ty: LiteralType,
    /// Evaluation order, including map keys before corresponding values.
    pub children: Vec<TypedLiteral>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedBinding {
    pub name: String,
    pub declared: Option<LiteralType>,
    pub value: TypedLiteral,
}

fn diag(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

fn builtin_name(name: &str) -> bool {
    name.starts_with("@builtin/")
}

/// Split only on top-level commas. Generic nesting is bounded independently.
fn type_args<'a>(input: &'a str, at: Pos, depth: usize) -> Result<Vec<&'a str>, Diagnostic> {
    if depth >= MAX_LITERAL_DEPTH {
        return Err(diag(at, "E_NESTING", "collection type nesting exceeds the limit"));
    }
    let mut parts = Vec::new();
    let mut level = 0usize;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '<' => {
                level += 1;
                if level > MAX_LITERAL_DEPTH {
                    return Err(diag(at, "E_NESTING", "collection type nesting exceeds the limit"));
                }
            }
            '>' => {
                level = level.checked_sub(1).ok_or_else(||
                    diag(at, "E_TYPE", "unbalanced collection generic argument"))?;
            }
            ',' if level == 0 => {
                parts.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if level != 0 {
        return Err(diag(at, "E_TYPE", "unbalanced collection generic argument"));
    }
    parts.push(&input[start..]);
    if parts.iter().any(|p| p.is_empty()) {
        return Err(diag(at, "E_TYPE_ARITY", "empty collection type argument"));
    }
    Ok(parts)
}

fn canonical_type(name: &str, pos: Pos, depth: usize) -> Result<LiteralType, Diagnostic> {
    if depth >= MAX_LITERAL_DEPTH {
        return Err(diag(pos, "E_NESTING", "collection type nesting exceeds the limit"));
    }
    let n = name.strip_prefix("@builtin/").unwrap_or(name);
    Ok(match n {
        "int" => LiteralType::Int,
        "bool" => LiteralType::Bool,
        "float" => LiteralType::Float,
        "string" => LiteralType::String,
        "Record" => LiteralType::OpenRecord,
        "Array" => LiteralType::Array(Box::new(LiteralType::Bottom)),
        "List" => LiteralType::List(Box::new(LiteralType::Bottom)),
        "Map" => LiteralType::Map(Box::new(LiteralType::Bottom), Box::new(LiteralType::Bottom)),
        "Tuple" => LiteralType::AnyTuple,
        _ => {
            let (head, rest) = n.split_once('<').ok_or_else(||
                diag(pos, "E_UNSUPPORTED_TYPE", format!("collection element type '{name}' is not in this static typing profile")))?;
            let inner = rest.strip_suffix('>').ok_or_else(||
                diag(pos, "E_TYPE", "collection type arguments must end with '>'"))?;
            let parts = type_args(inner, pos, depth + 1)?;
            let args: Vec<_> = parts.iter().map(|p| canonical_type(p, pos, depth + 1))
                .collect::<Result<_, _>>()?;
            match (head, args.as_slice()) {
                ("Array", [item]) => LiteralType::Array(Box::new(item.clone())),
                ("List", [item]) => LiteralType::List(Box::new(item.clone())),
                ("Map", [key, value]) =>
                    LiteralType::Map(Box::new(key.clone()), Box::new(value.clone())),
                ("Tuple", values) if !values.is_empty() => LiteralType::Tuple(values.to_vec()),
                _ => return Err(diag(pos, "E_TYPE_ARITY",
                    format!("unsupported collection constructor/arity in '{name}'"))),
            }
        }
    })
}

/// Parse a type annotation as far as the literal preflight needs. A user-defined
/// nominal type remains owned by the ordinary checker; no fake structural alias.
fn annotation(ty: Type, program: &Program, pos: Pos) -> Result<Option<LiteralType>, Diagnostic> {
    match ty {
        Type::Int => Ok(Some(LiteralType::Int)),
        Type::Bool => Ok(Some(LiteralType::Bool)),
        Type::Float => Ok(Some(LiteralType::Float)),
        Type::Str => Ok(Some(LiteralType::String)),
        Type::Named(i) => {
            let name = &program.type_names[i as usize].0;
            if builtin_name(name) {
                Ok(Some(canonical_type(name, pos, 0)?))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

fn compatible(actual: &LiteralType, expected: &LiteralType) -> bool {
    use LiteralType::*;
    match (actual, expected) {
        (_, Bottom) | (Bottom, _) => true,
        (Record(_), OpenRecord) | (OpenRecord, OpenRecord) => true,
        (Record(a), Record(b)) =>
            a.len() == b.len() && a.iter().zip(b).all(|((na,ta),(nb,tb))|
                na == nb && compatible(ta,tb)),
        (Array(a), Array(b)) | (List(a), List(b)) => compatible(a,b),
        (Map(ka, va), Map(kb, vb)) => compatible(ka,kb) && compatible(va,vb),
        (Tuple(_), AnyTuple) => true,
        (Tuple(a), Tuple(b)) =>
            a.len() == b.len() && a.iter().zip(b).all(|(x,y)| compatible(x,y)),
        _ => actual == expected,
    }
}

fn common(a: LiteralType, b: LiteralType, at: Pos) -> Result<LiteralType, Diagnostic> {
    use LiteralType::*;
    if a == Bottom { return Ok(b); }
    if b == Bottom { return Ok(a); }
    if a == b { return Ok(a); }
    match (a, b) {
        (Record(_), Record(_)) | (Record(_), OpenRecord) |
        (OpenRecord, Record(_)) => Ok(OpenRecord),
        (List(a), List(b)) => Ok(List(Box::new(common(*a, *b, at)?))),
        (Array(a), Array(b)) => Ok(Array(Box::new(common(*a, *b, at)?))),
        (Map(ka,va), Map(kb,vb)) => Ok(Map(
            Box::new(common(*ka,*kb,at)?), Box::new(common(*va,*vb,at)?))),
        (Tuple(a), Tuple(b)) if a.len() == b.len() =>
            Ok(Tuple(a.into_iter().zip(b).map(|(x,y)| common(x,y,at))
                .collect::<Result<Vec<_>,_>>()?)),
        (a,b) => Err(diag(at, "E_TYPE", format!(
            "collection contains incompatible element types {a:?} and {b:?}"))),
    }
}

fn matches_expected(actual: &LiteralType, expected: Option<&LiteralType>, at: Pos)
    -> Result<(), Diagnostic>
{
    if let Some(expected) = expected {
        if !compatible(actual, expected) {
            return Err(diag(at, "E_TYPE", format!(
                "literal type {actual:?} is not assignable to {expected:?}")));
        }
    }
    Ok(())
}

fn inferred(e: &Expr, expected: Option<&LiteralType>, depth: usize)
    -> Result<TypedLiteral, Diagnostic>
{
    use LiteralType::*;
    if depth >= MAX_LITERAL_DEPTH {
        return Err(diag(e.pos, "E_NESTING", "literal expression nesting exceeds the limit"));
    }
    let mut children = Vec::new();
    let ty = match &e.kind {
        ExprKind::Int(Some(n)) if *n <= i64::MAX as u64 => Int,
        ExprKind::Int(_) => return Err(diag(e.pos, "E_INTEGER_RANGE",
            "integer literal outside signed 64-bit range")),
        ExprKind::Float(_) => Float,
        ExprKind::Bool(_) => Bool,
        ExprKind::Str(_) => String,
        ExprKind::Unary(UnOp::Neg, v) if matches!(&v.kind, ExprKind::Int(_)) => {
            // The exact signed-minimum literal is represented as i64::MAX+1.
            if let ExprKind::Int(Some(n)) = &v.kind {
                if *n > (i64::MAX as u64) + 1 {
                    return Err(diag(v.pos, "E_INTEGER_RANGE", "signed int minimum exceeded"));
                }
            }
            Int
        }
        ExprKind::Unary(UnOp::Plus, v) if matches!(v.kind, ExprKind::Int(_)) =>
            inferred(v, Some(&Int), depth + 1)?.ty,
        ExprKind::Array(items) => {
            let (kind, element) = match expected {
                Some(Array(element)) => (true, element.as_ref().clone()),
                Some(List(element)) => (false, element.as_ref().clone()),
                None => (false, Bottom),
                _ => return Err(diag(e.pos, "E_TYPE", "bracket literal needs Array or List context")),
            };
            let mut found = Bottom;
            for item in items {
                let child = inferred(item,
                    if element == Bottom { None } else { Some(&element) }, depth + 1)?;
                found = common(found, child.ty.clone(), item.pos)?;
                children.push(child);
            }
            let element_ty = if element == Bottom { found } else { element };
            if kind { Array(Box::new(element_ty)) } else { List(Box::new(element_ty)) }
        }
        ExprKind::Record(fields) => {
            if matches!(expected, Some(Map(_,_) | Array(_) | List(_) | Tuple(_) | AnyTuple)) {
                return Err(diag(e.pos,"E_TYPE", "record literal requires Record context"));
            }
            let mut typed_fields = Vec::new();
            for (field, value) in fields {
                let child = inferred(value, None, depth + 1)?;
                typed_fields.push((field.clone(), child.ty.clone()));
                children.push(child);
            }
            Record(typed_fields)
        }
        ExprKind::Map(entries) => {
            let (wanted_k, wanted_v) = match expected {
                Some(Map(k,v)) => (k.as_ref().clone(), v.as_ref().clone()),
                None => (Bottom, Bottom),
                _ => return Err(diag(e.pos, "E_TYPE", "map literal requires Map context")),
            };
            let (mut keys, mut vals) = (Bottom, Bottom);
            for (key,value) in entries {
                let key_node = inferred(key, if wanted_k == Bottom { None } else { Some(&wanted_k) }, depth + 1)?;
                let value_node = inferred(value, if wanted_v == Bottom { None } else { Some(&wanted_v) }, depth + 1)?;
                keys = common(keys,key_node.ty.clone(),key.pos)?;
                vals = common(vals,value_node.ty.clone(),value.pos)?;
                children.push(key_node);
                children.push(value_node);
            }
            let key_ty = if wanted_k == Bottom { keys } else { wanted_k };
            let val_ty = if wanted_v == Bottom { vals } else { wanted_v };
            if !matches!(key_ty, Int | Bool | String | Bottom) {
                return Err(diag(e.pos, "E_MAP_KEY",
                    "this profile permits only scalar hashable map key types"));
            }
            Map(Box::new(key_ty), Box::new(val_ty))
        }
        ExprKind::EmptyBraces => match expected {
            Some(OpenRecord) | Some(Record(_)) => Record(Vec::new()),
            Some(Map(key,val)) if **key != Bottom && **val != Bottom =>
                Map(key.clone(), val.clone()),
            _ => return Err(diag(e.pos,"E_INFER",
                "empty '{}' needs an unambiguous Record or Map<K,V> context")),
        },
        ExprKind::Tuple(items) => {
            let slots = match expected {
                Some(Tuple(slots)) => {
                    if slots.len()!=items.len() {
                        return Err(diag(e.pos, "E_TYPE_ARITY",
                            format!("tuple has {} elements, expected {}", items.len(), slots.len())));
                    }
                    Some(slots)
                }
                Some(AnyTuple) | None => None,
                _ => return Err(diag(e.pos, "E_TYPE", "tuple literal requires Tuple context")),
            };
            let mut tys = Vec::new();
            for (i,item) in items.iter().enumerate() {
                let child = inferred(item,slots.map(|s| &s[i]),depth + 1)?;
                tys.push(child.ty.clone());
                children.push(child);
            }
            Tuple(tys)
        }
        _ => return Err(diag(e.pos, "E_UNSUPPORTED",
            "nonliteral collection elements require ownership-aware expression type checking")),
    };
    matches_expected(&ty,expected,e.pos)?;
    Ok(TypedLiteral { pos: e.pos,ty,children })
}

fn starts_literal(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Array(_) | ExprKind::Record(_) |
        ExprKind::Map(_) | ExprKind::EmptyBraces | ExprKind::Tuple(_))
}

/// Staged semantic preflight: checked declarations are *not* yet executable.
/// Scalar-only statements remain the responsibility of the existing checker.
fn walk(body: &[Stmt], program: &Program, out: &mut Vec<TypedBinding>)
    -> Result<(), Diagnostic>
{
    for stmt in body {
        match stmt {
            Stmt::Bind {name, ty,init,pos,..} => {
                let wanted = ty.map(|t| annotation(t,program,*pos)).transpose()?.flatten();
                if wanted.as_ref().is_some_and(|t| matches!(t, LiteralType::Array(_) |
                    LiteralType::List(_) | LiteralType::Map(_,_) | LiteralType::OpenRecord |
                    LiteralType::Record(_) | LiteralType::Tuple(_) | LiteralType::AnyTuple))
                    || starts_literal(init)
                {
                    let value = inferred(init,wanted.as_ref(),0)?;
                    if ty.is_none() && (matches!(&init.kind, ExprKind::EmptyBraces) ||
                        matches!(&init.kind, ExprKind::Array(v) if v.is_empty())) {
                        return Err(diag(init.pos,"E_INFER",
                            "empty collection without type annotation is ambiguous"));
                    }
                    out.push(TypedBinding {name:name.clone(),declared:wanted,value});
                }
            }
            Stmt::Block(inner) | Stmt::While {body:inner,..} |
            Stmt::Loop {body:inner,..} | Stmt::StateWrite {body:inner,..} |
            Stmt::ForIn {body:inner,..} => walk(inner,program,out)?,
            Stmt::If {arms,otherwise,..} => {
                for (_,inner) in arms { walk(inner,program,out)?; }
                if let Some(inner) = otherwise { walk(inner,program,out)?; }
            }
            Stmt::For {init,update,body,..} => {
                if let Some(s) = init { walk(std::slice::from_ref(s.as_ref()),program,out)?; }
                walk(body,program,out)?;
                if let Some(s) = update { walk(std::slice::from_ref(s.as_ref()),program,out)?; }
            }
            Stmt::Match {arms,..} => {
                for arm in arms { walk(&arm.body,program,out)?; }
            }
            Stmt::Try {body,catch,finally,..} => {
                walk(body,program,out)?;
                if let Some((_,inner)) = catch { walk(inner,program,out)?; }
                if let Some(inner) = finally { walk(inner,program,out)?; }
            }
            Stmt::Defer {body,..} | Stmt::Recover {body,..} =>
                walk(body,program,out)?,
            _ => {}
        }
    }
    Ok(())
}

/// Analyze local collection initializers. A successful response means static
/// literal typing only; the browser compiler still refuses guest codegen.
pub fn analyze(program: &Program) -> Result<Vec<TypedBinding>, Diagnostic> {
    let mut out = Vec::new();
    for func in &program.funcs {
        walk(&func.body,program,&mut out)?;
    }
    Ok(out)
}
