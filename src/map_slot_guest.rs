//! Bounded scalar Map guest profile. Literal keys are interned without hash
//! collisions; actual presence and value slots are initialized in fresh
//! heap storage. All writes go through the ordinary class ownership checker.
//! No arbitrary calls, escaping handles, dynamic keys, or control-flow
//! mutation joins are admitted. unwrap requires static presence proof.
use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const INTERNAL: &str = "$fixed_scalar_map";
const INDEX: &str = "$map_index_";
const MAX_KEYS: usize = 32;
type R<T> = Result<T, Diagnostic>;
type Keys = BTreeSet<String>;
type Fields = BTreeMap<String, Keys>;
type Roots = BTreeMap<String, Fields>;

fn deny(pos: Pos, msg: &str) -> Diagnostic { Diagnostic::new(pos, "E_COPY_UNPROVEN", msg) }
pub(crate) fn selected(t: &[Token]) -> bool {
    t.windows(2).any(|p| matches!((&p[0].tok,&p[1].tok),
        (Tok::Ident(n),Tok::Lt) if n == "Map"))
}
pub(crate) fn encoded_key(key: &str) -> String {
    let mut result = INDEX.to_owned();
    for b in key.as_bytes() { use std::fmt::Write; let _ = write!(result, "{b:02x}"); }
    result
}
fn value_field(k: &str) -> String { format!("$value_{k}") }
fn presence_field(k: &str) -> String { format!("$present_{k}") }
fn int(n: u64, pos: Pos) -> Expr { Expr { kind: ExprKind::Int(Some(n)), pos } }
fn member(base: Expr, field: String, pos: Pos) -> Expr {
    Expr { kind: ExprKind::Member(Box::new(base), field), pos }
}
fn known_name(n: &str, pos: Pos) -> R<()> {
    if n.contains('$') || n == "Map" { Err(deny(pos, "reserved Map storage name")) } else { Ok(()) }
}
fn factory(e: &mut Expr, keys: &Keys) -> R<Keys> {
    let ExprKind::New(name,args) = &mut e.kind else { return Err(deny(e.pos, "Map requires fresh literal")); };
    if name != INTERNAL || args.len() % 2 != 0 { return Err(deny(e.pos, "invalid Map literal")); }
    let mut entries = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let ExprKind::Str(key) = &pair[0].kind else { return Err(deny(pair[0].pos, "literal key required")); };
        let key = encoded_key(key);
        if !keys.contains(&key) || !crate::array_slot_guest::literal_int(&pair[1]) {
            return Err(deny(pair[1].pos, "Map literal requires bounded scalar entries"));
        }
        if entries.insert(key,pair[1].clone()).is_some() {
            return Err(Diagnostic::new(pair[0].pos,"E_DUPLICATE","duplicate Map key"));
        }
    }
    let present = entries.keys().cloned().collect();
    *args = keys.iter().flat_map(|k| [
        int(u64::from(entries.contains_key(k)),e.pos),
        entries.get(k).cloned().unwrap_or_else(|| int(0,e.pos)),
    ]).collect();
    Ok(present)
}
fn map_place<'a>(e: &'a Expr, roots: &Roots) -> R<(&'a str,String)> {
    let (root, field) = match &e.kind {
        ExprKind::Var(n) => (n.as_str(), String::new()),
        ExprKind::Member(base,f) => match &base.kind {
            ExprKind::Var(n) => (n.as_str(),f.clone()),
            _ => return Err(deny(e.pos,"Map place requires named root")),
        },
        _ => return Err(deny(e.pos,"Map place requires named root")),
    };
    if roots.get(root).is_none_or(|f| !f.contains_key(&field)) {
        return Err(deny(e.pos,"unproved Map receiver"));
    }
    Ok((root,field))
}
fn literal_get(e: &Expr, roots: &Roots, keys: &Keys) -> R<(Expr,String,bool)> {
    let ExprKind::MethodCall(base,method,args) = &e.kind else { return Err(deny(e.pos,"get(key) required")); };
    if method != "get" || args.len() != 1 { return Err(deny(e.pos,"get requires one literal key")); }
    let ExprKind::Str(k) = &args[0].kind else { return Err(deny(e.pos,"dynamic Map key")); };
    let k = encoded_key(k);
    let (root, field) = map_place(base,roots)?;
    if !keys.contains(&k) { return Err(deny(e.pos,"key outside closed Map shape")); }
    Ok((base.as_ref().clone(),k.clone(),roots[root][&field].contains(&k)))
}
fn scalar(e: &mut Expr, roots: &Roots, keys: &Keys) -> R<()> {
    let pos = e.pos;
    match &mut e.kind {
        ExprKind::Int(_) | ExprKind::Bool(_) => Ok(()),
        ExprKind::Var(n) if !roots.contains_key(n) => known_name(n,pos),
        ExprKind::Unary(_,a) => scalar(a,roots,keys),
        ExprKind::Binary(_,a,b) => { scalar(a,roots,keys)?; scalar(b,roots,keys) }
        ExprKind::Member(base,field) if matches!(&base.kind,ExprKind::Var(n) if roots.contains_key(n))
            && !field.contains('$') => {
                if let ExprKind::Var(n) = &base.kind {
                    if roots[n].contains_key(field) { return Err(deny(pos,"Map handle cannot escape")); }
                }
                Ok(())
            }
        ExprKind::MethodCall(base,method,args) if args.is_empty() && matches!(method.as_str(),"is_some"|"unwrap") => {
            let (receiver,key,present) = literal_get(base,roots,keys)?;
            if method == "unwrap" {
                if !present { return Err(deny(pos,"Map unwrap requires proven present key")); }
                *e = member(receiver,value_field(&key),pos);
            } else {
                e.kind = ExprKind::Binary(BinOp::Eq,
                    Box::new(member(receiver,presence_field(&key),pos)),Box::new(int(1,pos)));
            }
            Ok(())
        }
        _ => Err(deny(pos,"unproved Map scalar expression or escaping handle")),
    }
}
fn type_name(t: Option<Type>, names: &[(String,Pos)]) -> Option<&str> {
    if let Some(Type::Named(i)) = t { names.get(i as usize).map(|(n,_)| n.as_str()) } else { None }
}
fn body(stmts: &mut Vec<Stmt>, roots: &mut Roots, classes: &BTreeMap<String,Fields>,
    keys: &Keys, names: &[(String,Pos)]) -> R<()> {
    let mut out = Vec::new();
    for mut s in std::mem::take(stmts) {
        match &mut s {
            Stmt::Bind { kind,name,ty,init,pos } => {
                known_name(name,*pos)?;
                if roots.contains_key(name) { return Err(deny(*pos,"Map root shadowing is unsupported")); }
                let declared = type_name(*ty,names);
                let new_root = match &init.kind {
                    ExprKind::New(n,_) if n == INTERNAL && declared == Some(INTERNAL) => {
                        let present = factory(init,keys)?;
                        Some(BTreeMap::from([(String::new(),present)]))
                    }
                    ExprKind::New(n,args) if args.is_empty() && declared == Some(n.as_str()) =>
                        Some(classes.get(n).cloned().ok_or_else(|| deny(*pos,"unknown Map root class"))?),
                    ExprKind::Ownership(OwnershipOp::Copy | OwnershipOp::DeepCopy,false,a)
                        if declared.is_some() => {
                        let ExprKind::Var(source) = &a.kind else { return Err(deny(*pos,"copy requires named Map root")); };
                        let f = roots.get(source).ok_or_else(|| deny(*pos,"unproved copied root"))?;
                        Some(f.clone())
                    }
                    _ => { scalar(init,roots,keys)?; None }
                };
                if let Some(f) = new_root {
                    if *kind != BindKind::Let { return Err(deny(*pos,"Map root requires let binding")); }
                    roots.insert(name.clone(),f);
                } else if declared.is_some() { return Err(deny(*pos,"unproved named binding")); }
            }
            Stmt::FieldAssign { target,field,value,pos } if field.starts_with(INDEX) => {
                let (root,slot) = map_place(target,roots)?;
                if !keys.contains(field) || !crate::array_slot_guest::literal_int(value) {
                    return Err(deny(*pos,"Map insert requires a bounded literal key and int value"));
                }
                let root = root.to_owned();
                roots.get_mut(&root).unwrap().get_mut(&slot).unwrap().insert(field.clone());
                // Both stores are checked against the same unchanged authority.
                // Integer literal stores cannot suspend or fail between writes.
                out.push(Stmt::FieldAssign { target:target.clone(), field:value_field(field), value:value.clone(),pos:*pos });
                out.push(Stmt::FieldAssign { target:target.clone(), field:presence_field(field), value:int(1,*pos),pos:*pos });
                continue;
            }
            Stmt::FieldAssign { target,field,value,pos } => {
                if !matches!(&target.kind, ExprKind::Var(n) if roots.get(n).is_some_and(|f| !f.contains_key(field)))
                    || field.contains('$') || !crate::array_slot_guest::literal_int(value) {
                    return Err(deny(*pos,"only scalar outer slot writes admitted"));
                }
            }
            Stmt::If { arms, otherwise, pos } => {
                // Return-only branches cannot change the compile-time
                // presence proof or ownership state of the following path.
                if arms.iter().any(|(_,b)| !matches!(b.as_slice(),[Stmt::Return(Some(_),_)]))
                    || otherwise.as_ref().is_some_and(|b| !matches!(b.as_slice(),[Stmt::Return(Some(_),_)])) {
                    return Err(deny(*pos,"Map conditional must be return-only"));
                }
                for (cond,b) in arms { scalar(cond,roots,keys)?; body(b,&mut roots.clone(),classes,keys,names)?; }
                if let Some(b) = otherwise { body(b,&mut roots.clone(),classes,keys,names)?; }
            }
            Stmt::Return(Some(e),_) => scalar(e,roots,keys)?,
            _ => return Err(deny(Pos::default(),"unsupported Map control flow or ownership operation")),
        }
        out.push(s);
    }
    *stmts = out;
    Ok(())
}
pub(crate) fn lower(mut p: Program,tokens:&[Token]) -> R<Program> {
    let pos = p.funcs.first().map_or(Pos::default(),|f| f.pos);
    if p.funcs.len()!=1 || p.funcs[0].name!="main" || !p.funcs[0].params.is_empty()
        || p.funcs[0].ret!=Type::Int || p.funcs[0].is_generator || p.funcs[0].is_pure
        || p.funcs[0].declared_effects.is_some() || !p.actors.is_empty() || !p.enums.is_empty()
        || p.actor_interface.is_some() || !p.array_types.is_empty() {
        return Err(deny(pos,"Map guest requires ordinary classes and one int main"));
    }
    let mut keys=Keys::new();
    for token in tokens {
        if let Tok::Str(k)=&token.tok {
            if k.len()>256 { return Err(deny(token.pos,"Map key too long")); }
            keys.insert(encoded_key(k));
            if keys.len()>MAX_KEYS { return Err(deny(token.pos,"Map key-set budget exceeded")); }
        }
    }
    if keys.is_empty() { return Err(deny(pos,"empty closed Map key set")); }
    let names=&p.type_names;
    let mut classes=BTreeMap::new();
    for c in &mut p.classes {
        known_name(&c.name,c.pos)?;
        if c.state_container || c.ctor.is_some() || !c.methods.is_empty() {
            return Err(deny(c.pos,"Map root must be a closed constructor-free class"));
        }
        let mut fields=Fields::new();
        for f in &mut c.fields {
            known_name(&f.name,f.pos)?;
            let init=f.init.as_mut().ok_or_else(||deny(f.pos,"field initializer required"))?;
            if type_name(Some(f.ty),names)==Some(INTERNAL) {
                if !f.referent_mutable || !matches!(f.binding,FieldBinding::Let|FieldBinding::Val) {
                    return Err(deny(f.pos,"Map field requires explicit mutable referent"));
                }
                fields.insert(f.name.clone(),factory(init,&keys)?);
            } else if !matches!(f.ty,Type::Int|Type::Bool) || f.referent_mutable
                || !(crate::array_slot_guest::literal_int(init)||matches!(init.kind,ExprKind::Bool(_))) {
                return Err(deny(f.pos,"unsupported Map containing field"));
            }
        }
        if classes.insert(c.name.clone(),fields).is_some() { return Err(deny(c.pos,"duplicate class")); }
    }
    body(&mut p.funcs[0].body,&mut Roots::new(),&classes,&keys,names)?;
    p.classes.push(ClassDecl {
        name:INTERNAL.to_owned(),pos,public:false,state_container:false,ctor:None,methods:Vec::new(),
        fields:keys.iter().flat_map(|k|[presence_field(k),value_field(k)]).map(|name|Field {
            name,pos,ty:Type::Int,visibility:FieldVisibility::Public,mode:FieldMode::Constructor,
            binding:FieldBinding::Let,referent_mutable:false,init:Some(int(0,pos)),
        }).collect(),
    });
    Ok(p)
}
