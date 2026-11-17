//! Contextual typing for inline collection values (parser/semantic admission only).
//!
//! This module deliberately does NOT allocate, lower or emit guest-Wasm values.
//! Its output is an owned, structurally typed IR, separate from the executable
//! scalar checker. Unsupported expressions fail closed rather than being given
//! an invented runtime type. Once this is wired to checked guest allocation the
//! same type and tree structure can be consumed by the lowering stage.

use crate::ast::{BinOp, Expr, ExprKind, Pattern, Program, Stmt, Type, UnOp};
use std::collections::BTreeMap;
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

/// Reject invalid hash-key types at declaration time, including empty maps
/// that would otherwise bypass entry-by-entry key checking.
fn validate_hash_contract(ty: &LiteralType, at: Pos, depth: usize) -> Result<(), Diagnostic> {
    use LiteralType::*;
    if depth >= MAX_LITERAL_DEPTH {
        return Err(diag(at,"E_NESTING","collection type nesting exceeds the limit"));
    }
    match ty {
        Map(key, value) => {
            if !matches!(key.as_ref(), Int | Bool | String | Bottom) {
                return Err(diag(at, "E_MAP_KEY",
                    "this browser profile requires hashable scalar Map keys (int, bool, string)"));
            }
            validate_hash_contract(key, at, depth + 1)?;
            validate_hash_contract(value, at, depth + 1)
        }
        Array(item) | List(item) => validate_hash_contract(item, at, depth + 1),
        Record(fields) => {
            for (_, field_type) in fields {
                validate_hash_contract(field_type, at, depth + 1)?;
            }
            Ok(())
        }
        Tuple(items) => {
            for item in items {
                validate_hash_contract(item, at, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
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
                let ty = canonical_type(name, pos, 0)?;
                validate_hash_contract(&ty, pos, 0)?;
                Ok(Some(ty))
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

fn inferred(e: &Expr, expected: Option<&LiteralType>, depth: usize,
    locals: &BTreeMap<String, LiteralType>)
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
        ExprKind::Var(name) => {
            let ty = locals.get(name).ok_or_else(|| diag(e.pos, "E_UNBOUND",
                format!("collection element '{name}' has no verified local type")))?;
            // A bare variable here would alias an owned collection: require an
            // explicit move/deep copy after the ownership checker is integrated.
            if owns_storage(ty) {
                return Err(diag(e.pos,"E_OWNERSHIP",
                    format!("owned collection '{name}' needs an explicit checked transfer or copy")));
            }
            ty.clone()
        }
        ExprKind::Member(base, field) => {
            let base_type = match &base.kind {
                ExprKind::Var(name) => locals.get(name).cloned().ok_or_else(||
                    diag(base.pos,"E_UNBOUND",
                        format!("record receiver '{name}' has no verified local type")))?,
                _ => {
                    let item = inferred(base,None,depth+1,locals)?;
                    children.push(item.clone());
                    item.ty
                }
            };
            let field_ty = static_record_field_type(&base_type,field,e.pos)?;
            if owns_storage(&field_ty) {
                return Err(diag(e.pos,"E_OWNERSHIP",
                    "reading a nested owned field needs an explicit borrow, move or deep copy"));
            }
            field_ty
        }
        ExprKind::Binary(op, left, right) if matches!(op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) => {
            let left_node = inferred(left,None,depth+1,locals)?;
            let right_node = inferred(right,None,depth+1,locals)?;
            // Match the existing executable checker: mixed numeric operands
            // widen to float, and String + String concatenates. Do not assume
            // int / int produces a value (the guest checker rejects it).
            let result = match (&left_node.ty, &right_node.ty) {
                (String, String) if *op == BinOp::Add => String,
                (Int, Int) if *op == BinOp::Div => {
                    return Err(diag(e.pos,"E_TYPE", "int / int has no admitted result type"));
                }
                (Int, Int) => Int,
                (Int | Float, Int | Float) => Float,
                _ => return Err(diag(e.pos,"E_TYPE",
                    "arithmetic requires compatible numeric operands or two strings with '+'")),
            };
            children.push(left_node);
            children.push(right_node);
            result
        }
        ExprKind::Unary(UnOp::Neg, v) if matches!(&v.kind, ExprKind::Int(_)) => {
            // The exact signed-minimum literal is represented as i64::MAX+1.
            if let ExprKind::Int(Some(n)) = &v.kind {
                if *n > (i64::MAX as u64) + 1 {
                    return Err(diag(v.pos, "E_INTEGER_RANGE", "signed int minimum exceeded"));
                }
            }
            Int
        }
        ExprKind::Unary(UnOp::Neg | UnOp::Plus, v) => {
            let child = inferred(v,None,depth+1,locals)?;
            if !matches!(&child.ty, Int | Float) {
                return Err(diag(e.pos,"E_TYPE",
                    "unary +/- requires int or float"));
            }
            // A source-signed minimum integer is handled above. All other
            // unary numeric operations retain the checked scalar type.
            let ty = child.ty.clone();
            children.push(child);
            ty
        }
        ExprKind::Unary(UnOp::Not, v) => {
            let child = inferred(v,Some(&Bool),depth+1,locals)?;
            children.push(child);
            Bool
        },
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
                    if element == Bottom { None } else { Some(&element) }, depth + 1, locals)?;
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
                let child = inferred(value, None, depth + 1, locals)?;
                typed_fields.push((field.clone(), child.ty.clone()));
                children.push(child);
            }
            // A Record is structurally keyed by its field-name/type set,
            // not by the textual order of entries. Preserve *evaluation*
            // order independently in `children`, but canonicalize the type
            // shape for deterministic identity and AOT shape interning.
            typed_fields.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
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
                let key_node = inferred(key, if wanted_k == Bottom { None } else { Some(&wanted_k) }, depth + 1, locals)?;
                let value_node = inferred(value, if wanted_v == Bottom { None } else { Some(&wanted_v) }, depth + 1, locals)?;
                keys = common(keys,key_node.ty.clone(),key.pos)?;
                vals = common(vals,value_node.ty.clone(),value.pos)?;
                children.push(key_node);
                children.push(value_node);
            }
            let key_ty = if wanted_k == Bottom { keys } else { wanted_k };
            let val_ty = if wanted_v == Bottom { vals } else { wanted_v };
            if !matches!(&key_ty, Int | Bool | String | Bottom) {
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
                let child = inferred(item,slots.map(|s| &s[i]),depth + 1, locals)?;
                tys.push(child.ty.clone());
                children.push(child);
            }
            Tuple(tys)
        }
        _ => return Err(diag(e.pos, "E_UNSUPPORTED",
            "nonliteral collection elements require ownership-aware expression type checking")),
    };
    validate_hash_contract(&ty,e.pos,0)?;
    matches_expected(&ty,expected,e.pos)?;
    Ok(TypedLiteral { pos: e.pos,ty,children })
}

fn owns_storage(ty: &LiteralType) -> bool {
    matches!(ty, LiteralType::OpenRecord | LiteralType::Record(_) |
        LiteralType::Array(_) | LiteralType::List(_) | LiteralType::Map(_,_) |
        LiteralType::Tuple(_) | LiteralType::AnyTuple)
}

/// Static typed-record field lookup, separate from eventual guest indexing.
/// Closed records reject a missing name; bare open Record needs a proven
/// presence/type guard. This helper does not certify guest field access.
pub fn static_record_field_type(
    ty: &LiteralType, field: &str, at: Pos
) -> Result<LiteralType, Diagnostic> {
    match ty {
        LiteralType::Record(fields) => fields.iter()
            .find(|(name, _)| name == field)
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| diag(at, "E_NO_FIELD",
                format!("closed record has no field '{field}'"))),
        LiteralType::OpenRecord => Err(diag(at, "E_FIELD_PRESENCE",
            format!("open Record field '{field}' requires a verified presence/type guard"))),
        _ => Err(diag(at, "E_TYPE", "static record selection requires Record type")),
    }
}

fn starts_literal(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Array(_) | ExprKind::Record(_) |
        ExprKind::Map(_) | ExprKind::EmptyBraces | ExprKind::Tuple(_))
}

/// A tracked owned variable used as a bare expression would require an
/// ownership operation that the browser backend cannot lower yet.
fn references_owned(e: &Expr, locals: &BTreeMap<String, LiteralType>) -> bool {
    match &e.kind {
        ExprKind::Var(name) => locals.get(name).is_some_and(owns_storage),
        ExprKind::Member(base, _) => match &base.kind {
            ExprKind::Var(name) => locals.get(name).is_some_and(owns_storage),
            other => matches!(other, ExprKind::Record(_)),
        },
        _ => false,
    }
}

fn scalar_seed(e: &Expr, locals: &BTreeMap<String, LiteralType>) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) |
        ExprKind::Str(_) => true,
        ExprKind::Unary(UnOp::Neg | UnOp::Plus | UnOp::Not, inner) =>
            scalar_seed(inner, locals),
        ExprKind::Var(name) => locals.get(name).is_some_and(|t| !owns_storage(t)),
        ExprKind::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, a, b) =>
            scalar_seed(a,locals) && scalar_seed(b,locals),
        _ => false,
    }
}

/// A scope-aware semantic preflight. Local type identities are conserved through
/// sequential statements and lexical scopes, never propagated blindly across
/// conditionals, loops or exception edges. Unknown expressions defer to the
/// production checker; unknown expressions used in collection values reject.
fn walk(body: &[Stmt], program: &Program,
    locals: &mut BTreeMap<String, LiteralType>, out: &mut Vec<TypedBinding>)
    -> Result<(), Diagnostic>
{
    for stmt in body {
        match stmt {
            Stmt::Bind {name, ty,init,pos,..} => {
                let wanted = ty.map(|t| annotation(t,program,*pos)).transpose()?.flatten();
                let collection_context = wanted.as_ref().is_some_and(owns_storage);
                let inspect = collection_context || starts_literal(init) ||
                    references_owned(init,locals);
                let is_seed = scalar_seed(init,locals);
                let checked = if inspect || is_seed {
                    Some(inferred(init,wanted.as_ref(),0,locals)?)
                } else { None };
                if ty.is_none() && (matches!(&init.kind, ExprKind::EmptyBraces) ||
                    matches!(&init.kind, ExprKind::Array(items) if items.is_empty())) {
                    return Err(diag(init.pos,"E_INFER",
                        "empty collection without type annotation is ambiguous"));
                }
                if inspect {
                    let value = checked.clone().expect("inspected literal must be checked");
                    out.push(TypedBinding {name:name.clone(),declared:wanted.clone(),value});
                }
                let inferred_ty = checked.map(|item| item.ty);
                // An explicit open Record or collection annotation remains the
                // local's declared contract, even when its initializer was
                // more specific. Prevents unguarded field assumptions after
                // reassignment and preserves closed shape only for inferred
                // bindings.
                if let Some(stored) = wanted.or(inferred_ty) {
                    locals.insert(name.clone(), stored);
                } else {
                    locals.remove(name);
                }
            }
            Stmt::FieldAssign {target,pos,..} => {
                if let ExprKind::Var(name) = &target.kind {
                    if locals.get(name).is_some_and(owns_storage) {
                        return Err(diag(*pos,"E_OWNERSHIP",
                            format!("mutating owned record '{name}' requires a checked field-write capability")));
                    }
                }
            }
            Stmt::Assign {name,value,pos} => {
                if locals.get(name).is_some_and(owns_storage) {
                    return Err(diag(*pos,"E_OWNERSHIP",
                        format!("reassignment of owned collection '{name}' needs ownership-aware flow checking")));
                }
                // A scalar assignment cannot change its declared type in the
                // existing checker. Retain only already verified scalar types.
                if let Some(known) = locals.get(name).cloned() {
                    if scalar_seed(value,locals) {
                        inferred(value,Some(&known),0,locals)?;
                    }
                }
            }
            Stmt::Block(inner) | Stmt::StateWrite {body:inner,..} => {
                walk(inner,program,&mut locals.clone(),out)?;
            }
            Stmt::While {body:inner,..} | Stmt::Loop {body:inner,..} => {
                walk(inner,program,&mut locals.clone(),out)?;
            }
            Stmt::ForIn {name,body,..} => {
                let mut child = locals.clone();
                child.remove(name);
                walk(body,program,&mut child,out)?;
            }
            Stmt::If {arms,otherwise,..} => {
                for (_,inner) in arms {
                    walk(inner,program,&mut locals.clone(),out)?;
                }
                if let Some(inner) = otherwise {
                    walk(inner,program,&mut locals.clone(),out)?;
                }
            }
            Stmt::For {init,update,body,..} => {
                let mut loop_env = locals.clone();
                if let Some(s) = init {
                    walk(std::slice::from_ref(s.as_ref()),program,&mut loop_env,out)?;
                }
                walk(body,program,&mut loop_env,out)?;
                if let Some(s) = update {
                    walk(std::slice::from_ref(s.as_ref()),program,&mut loop_env,out)?;
                }
            }
            Stmt::Match {arms,..} => {
                for arm in arms {
                    let mut child = locals.clone();
                    if let Pattern::Bind(name) = &arm.pattern {
                        child.remove(name);
                    }
                    walk(&arm.body,program,&mut child,out)?;
                }
            }
            Stmt::Try {body,catch,finally,..} => {
                walk(body,program,&mut locals.clone(),out)?;
                if let Some((name,inner)) = catch {
                    let mut child = locals.clone();
                    child.remove(name);
                    walk(inner,program,&mut child,out)?;
                }
                if let Some(inner) = finally {
                    walk(inner,program,&mut locals.clone(),out)?;
                }
            }
            Stmt::Defer {body,..} => walk(body,program,&mut locals.clone(),out)?,
            Stmt::Recover {body,err,cursor,..} => {
                let mut child = locals.clone();
                child.remove(err);
                child.remove(cursor);
                walk(body,program,&mut child,out)?;
            }
            Stmt::Return(Some(value),_) if references_owned(value,locals) => {
                // Return an owned object only after the language's move/borrow
                // checker has determined caller/callee ownership transfer.
                if matches!(&value.kind, ExprKind::Var(_)) {
                    return Err(diag(value.pos,"E_OWNERSHIP",
                        "returning an owned local requires explicit checked transfer"));
                }
                inferred(value,None,0,locals)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Analyze checked local collection initializers and scalar dependencies.
/// Success means type preflight only; browser guest allocation is still blocked.
pub fn analyze(program: &Program) -> Result<Vec<TypedBinding>, Diagnostic> {
    let mut out = Vec::new();
    for func in &program.funcs {
        let mut locals = BTreeMap::new();
        for param in &func.params {
            if let Some(ty) = annotation(param.ty,program,param.pos)? {
                locals.insert(param.name.clone(),ty);
            }
        }
        walk(&func.body,program,&mut locals,&mut out)?;
    }
    Ok(out)
}
