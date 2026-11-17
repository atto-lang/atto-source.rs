//! Bounded executable closed-Record lowering for the browser Wasm backend.
//!
//! This pass runs AFTER the existing structural type/ownership preflights and
//! BEFORE the ordinary typed class checker. It uses the checked class heap,
//! constructors and static field offsets; no raw pointers or JS execution.
//! Only flat, statically typed, identity-free scalar fields are admitted.
//! All other shapes (including quoted nonidentifier keys, nested mutable
//! graphs and open Record) retain an explicit unsupported diagnostic.
use std::collections::BTreeMap;

use crate::ast::{
    BinOp, BindKind, ClassDecl, Constructor, Expr, ExprKind, Field, FieldMode,
    Func, Param, Program, Stmt, Type, UnOp,
};
use crate::diag::{Diagnostic, Pos};

type R<T> = Result<T, Diagnostic>;
const MAX_FIELDS: usize = 8;
const MAX_SHAPES: usize = 48;
const MAX_EXPR_DEPTH: usize = 48;
const PREFIX: &str = "__atto_shape_with_";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Shape(Vec<(String, Type)>);

#[derive(Clone)]
enum Value {
    Scalar(Type),
    Record(Shape),
    Unknown,
}

type Frames = Vec<BTreeMap<String, Value>>;

fn flatten(frames: &Frames) -> BTreeMap<String, Value> {
    let mut visible = BTreeMap::new();
    for frame in frames {
        for (name, value) in frame {
            visible.insert(name.clone(), value.clone());
        }
    }
    visible
}

fn field_slot(shape: &Shape, field: &str, pos: Pos) -> R<String> {
    shape.0.iter().position(|(name, _)| name == field)
        .map(|index| format!("f{index}"))
        .ok_or_else(|| err(pos,"E_NO_FIELD",format!("Record has no field '{field}'")))
}

fn err(pos: Pos, code: &'static str, what: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, what)
}

fn is_scalar(t: Type) -> bool {
    matches!(t, Type::Int | Type::Bool | Type::Float | Type::Str)
}

fn expression(kind: ExprKind, pos: Pos) -> Expr { Expr { kind, pos } }

const MAX_KEY_BYTES: usize = 128;

fn initializer(ty: Type, pos: Pos) -> Expr {
    let kind = match ty {
        Type::Int => ExprKind::Int(Some(0)),
        Type::Bool => ExprKind::Bool(false),
        Type::Float => ExprKind::Float(0f64.to_bits()),
        Type::Str => ExprKind::Str(String::new()),
        _ => unreachable!("scalar shape validated"),
    };
    expression(kind, pos)
}

struct Lower {
    shapes: Vec<(Shape, String)>,
    hidden: usize,
    classes: Vec<ClassDecl>,
}

impl Lower {
    fn new() -> Self {
        Self { shapes: Vec::new(), hidden: 0, classes: Vec::new() }
    }
    fn fresh(&mut self) -> String {
        let n = self.hidden;
        self.hidden += 1;
        format!("{PREFIX}tmp_{n}")
    }
    fn class(&mut self, shape: &Shape, pos: Pos) -> R<String> {
        if let Some((_,name)) = self.shapes.iter().find(|(s,_)| s == shape) {
            return Ok(name.clone());
        }
        if self.shapes.len() >= MAX_SHAPES {
            return Err(err(pos, "E_SOURCE_LIMIT", "too many closed Record shape specializations"));
        }
        if shape.0.is_empty() || shape.0.len() > MAX_FIELDS {
            return Err(err(pos, "E_UNSUPPORTED", "guest Record needs 1..8 scalar fields"));
        }
        for (field, ty) in &shape.0 {
            if field.len() > MAX_KEY_BYTES {
                return Err(err(pos, "E_SOURCE_LIMIT", "guest Record label exceeds 128 UTF-8 bytes"));
            }
            if !is_scalar(*ty) {
                return Err(err(pos, "E_WITH_OWNERSHIP",
                    format!("field '{field}' is not a verified flat scalar")));
            }
        }
        let name = format!("{PREFIX}record_{}", self.shapes.len());
        let fields = shape.0.iter().enumerate().map(|(index,(_,ty))| Field {
            name: format!("f{index}"), pos, ty: *ty, public: true,
            mode: FieldMode::Constructor, init: initializer(*ty,pos),
        }).collect::<Vec<_>>();
        let mut params = Vec::new();
        let mut body = Vec::new();
        for (index,(_,ty)) in shape.0.iter().enumerate() {
            let parameter = format!("{PREFIX}arg_{index}");
            params.push(Param {name:parameter.clone(),ty:*ty,mutable:false,pos});
            body.push(Stmt::FieldAssign {
                target: expression(ExprKind::SelfRef,pos), field:format!("f{index}"),
                value: expression(ExprKind::Var(parameter),pos),pos,
            });
        }
        let cls = ClassDecl {
            name:name.clone(),pos,public:false,state_container:false,fields,
            ctor:Some(Constructor {pos,params,body}),methods:Vec::new()
        };
        self.shapes.push((shape.clone(),name.clone()));
        self.classes.push(cls);
        Ok(name)
    }
    fn inferred_scalar(&self, e: &Expr, env: &BTreeMap<String,Value>, depth: usize) -> R<Type> {
        if depth > MAX_EXPR_DEPTH {
            return Err(err(e.pos,"E_NESTING","Record value expression too deep"));
        }
        let ty = match &e.kind {
            ExprKind::Int(Some(n)) if *n <= i64::MAX as u64 => Type::Int,
            ExprKind::Int(_) => return Err(err(e.pos, "E_INTEGER_RANGE", "integer literal out of i64 range")),
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Var(name) => match env.get(name) {
                Some(Value::Scalar(t)) => *t,
                _ => return Err(err(e.pos,"E_UNSUPPORTED",
                    format!("Record field local '{name}' is not a proven scalar"))),
            },
            ExprKind::Member(recv, field) => match &recv.kind {
                ExprKind::Var(name) => match env.get(name) {
                    Some(Value::Record(shape)) => shape.0.iter()
                        .find(|(n,_)| n == field).map(|(_,t)| *t)
                        .ok_or_else(||err(e.pos,"E_NO_FIELD",format!("Record has no field '{field}'")))?,
                    _ => return Err(err(e.pos,"E_UNSUPPORTED",
                        "Record field projection needs a known local receiver")),
                },
                _ => return Err(err(e.pos,"E_UNSUPPORTED",
                    "guest Record projections need a statically known local receiver")),
            },
            ExprKind::Unary(op, inner) => {
                if *op == UnOp::Neg && matches!(&inner.kind,ExprKind::Int(None)) {
                    Type::Int
                } else {
                    let it=self.inferred_scalar(inner,env,depth+1)?;
                    match (op,it) {
                        (UnOp::Neg | UnOp::Plus,Type::Int | Type::Float) => it,
                        (UnOp::Not,Type::Bool) => Type::Bool,
                        _ => return Err(err(e.pos,"E_TYPE","invalid scalar unary update")),
                    }
                }
            }
            ExprKind::Binary(op,a,b) => {
                let lhs=self.inferred_scalar(a,env,depth+1)?;
                let rhs=self.inferred_scalar(b,env,depth+1)?;
                match (op,lhs,rhs) {
                    (BinOp::Add | BinOp::Sub | BinOp::Mul,Type::Int,Type::Int) => Type::Int,
                    (BinOp::Add | BinOp::Sub | BinOp::Mul,Type::Float,Type::Int | Type::Float)
                    | (BinOp::Add | BinOp::Sub | BinOp::Mul,Type::Int,Type::Float) => Type::Float,
                    (BinOp::Add,Type::Str,Type::Str) => Type::Str,
                    (BinOp::Eq | BinOp::Ne,Type::Int,Type::Int)
                    | (BinOp::Eq | BinOp::Ne,Type::Bool,Type::Bool)
                    | (BinOp::Eq | BinOp::Ne,Type::Str,Type::Str) => Type::Bool,
                    (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,Type::Int,Type::Int) => Type::Bool,
                    _ => return Err(err(e.pos,"E_UNSUPPORTED",
                        "guest Record scalar operator/type combination not supported")),
                }
            },
            _ => return Err(err(e.pos,"E_UNSUPPORTED",
                "guest Record field must be a statically typed scalar; no dynamic call, mutable graph or ownership escape")),
        };
        Ok(ty)
    }
    fn temporary(&mut self, pre: &mut Vec<Stmt>, init: Expr, ty: Option<Type>) -> Expr {
        let pos=init.pos;
        let name=self.fresh();
        pre.push(Stmt::Bind {kind:BindKind::Val,name:name.clone(),ty,init,pos});
        expression(ExprKind::Var(name),pos)
    }
    fn lower_record(&mut self, e: Expr, env: &BTreeMap<String,Value>, depth: usize)
        -> R<(Vec<Stmt>,Expr,Shape)>
    {
        if depth > MAX_EXPR_DEPTH {
            return Err(err(e.pos,"E_NESTING","nested with expressions exceed guest budget"));
        }
        let pos=e.pos;
        match e.kind {
            ExprKind::Record(fields) => {
                if fields.is_empty() || fields.len()>MAX_FIELDS {
                    return Err(err(pos,"E_UNSUPPORTED","guest Record requires 1..8 scalar fields"));
                }
                let mut pre = Vec::new();
                let mut slots = BTreeMap::<String,(Type,Expr)>::new();
                for (field,value) in fields {
                    if slots.contains_key(&field) {return Err(err(value.pos,"E_DUPLICATE","duplicate Record field"));}
                    let ty=self.inferred_scalar(&value,env,0)?;
                    // A source-order temporary prevents canonical field sorting
                    // from reordering or duplicating any initializer.
                    let temp=self.temporary(&mut pre,value,Some(ty));
                    slots.insert(field,(ty,temp));
                }
                let shape=Shape(slots.iter().map(|(name,(ty,_))|(name.clone(),*ty)).collect());
                let name=self.class(&shape,pos)?;
                let args=slots.into_values().map(|(_,expr)|expr).collect();
                Ok((pre,expression(ExprKind::New(name,args),pos),shape))
            }
            ExprKind::With(base,updates) => {
                let (mut pre,original_expr,original) = match base.kind {
                    ExprKind::Var(name) => match env.get(&name) {
                        Some(Value::Record(shape)) =>
                            (Vec::new(),expression(ExprKind::Var(name),base.pos),shape.clone()),
                        Some(Value::Scalar(_))=>return Err(err(pos,"E_WITH_TYPE","with receiver is not a Record")),
                        None=>return Err(err(pos,"E_WITH_UNRESOLVED","with needs a known source Record")),
                    },
                    other => self.lower_record(expression(other,base.pos),env,depth+1)?,
                };
                let receiver = if matches!(original_expr.kind, ExprKind::Var(_)) {
                    original_expr
                } else {
                    // Evaluates computed source exactly once. Only the newly
                    // constructed intermediate Record is bound, never moved
                    // from an existing source local.
                    self.temporary(&mut pre,original_expr,None)
                };
                let mut updated = BTreeMap::<String,(Type,Expr)>::new();
                for (field,value) in updates {
                    if updated.contains_key(&field) {
                        return Err(err(value.pos,"E_DUPLICATE","duplicate with update field"));
                    }
                    let ty=self.inferred_scalar(&value,env,0)?;
                    let temp=self.temporary(&mut pre,value,Some(ty));
                    updated.insert(field,(ty,temp));
                }
                let mut merged=BTreeMap::<String,(Type,Expr)>::new();
                for (field,ty) in &original.0 {
                    if !updated.contains_key(field) {
                        merged.insert(field.clone(),(*ty,expression(
                            ExprKind::Member(Box::new(receiver.clone()),field.clone()),pos)));
                    }
                }
                for (field,pair) in updated { merged.insert(field,pair); }
                if merged.len()>MAX_FIELDS {
                    return Err(err(pos,"E_SOURCE_LIMIT","resulting guest Record exceeds 8 fields"));
                }
                let shape=Shape(merged.iter().map(|(name,(ty,_))|(name.clone(),*ty)).collect());
                let name=self.class(&shape,pos)?;
                let args=merged.into_values().map(|(_,expr)|expr).collect();
                Ok((pre,expression(ExprKind::New(name,args),pos),shape))
            }
            _ => Err(err(pos,"E_UNSUPPORTED",
                "guest with source must be a known Record local or closed Record/with expression")),
        }
    }
    fn lower_func(&mut self, f: &mut Func, type_names:&[(String,Pos)]) -> R<()> {
        let mut env=BTreeMap::<String,Value>::new();
        for param in &f.params {
            if is_scalar(param.ty) {env.insert(param.name.clone(),Value::Scalar(param.ty));}
        }
        let mut transformed=Vec::new();
        for stmt in std::mem::take(&mut f.body) {
            match stmt {
                Stmt::Bind {kind,name,ty,init,pos} => {
                    if matches!(init.kind,ExprKind::Record(_) | ExprKind::With(_,_)) {
                        if kind == BindKind::Const {
                            return Err(err(pos,"E_CONST",
                                "runtime heap-allocated Record cannot initialize compile-time const"));
                        }
                        let (mut pre, expr,shape)=self.lower_record(init,&env,0)?;
                        if let Some(t)=ty {
                            let built_in=match t {
                                Type::Named(i)=>type_names.get(i as usize)
                                    .is_some_and(|(name,_)|name=="@builtin/Record"),
                                _=>false,
                            };
                            if !built_in {
                                return Err(err(pos,"E_TYPE",
                                    "executable Record requires an unannotated or Record-typed binding"));
                            }
                        }
                        transformed.append(&mut pre);
                        // The static preflight already checked the source's
                        // Record annotation; guest representation is private.
                        transformed.push(Stmt::Bind {kind,name:name.clone(),ty:None,init:expr,pos});
                        env.insert(name,Value::Record(shape));
                    } else {
                        if let Some(t)=ty {
                            if is_scalar(t) {env.insert(name.clone(),Value::Scalar(t));}
                        } else if let Ok(t)=self.inferred_scalar(&init,&env,0) {
                            env.insert(name.clone(),Value::Scalar(t));
                        }
                        transformed.push(Stmt::Bind {kind,name,ty,init,pos});
                    }
                }
                // Complex flow is still admitted by the ordinary compiler,
                // but Record/with expressions in it remain fail-closed unless
                // a later shape-aware CFG lowering proves their lifetime.
                other => transformed.push(other),
            }
        }
        f.body=transformed;
        Ok(())
    }
}

/// Rewrite only flat closed Records to real constructor/field access IR.
/// No source-level object identity, dynamic dictionary, open-record access,
/// partial publication, actor transport or nested heap graphs are invented.
pub fn lower(input: &Program) -> R<Program> {
    let mut p=input.clone();
    let mut lower=Lower::new();
    for f in &mut p.funcs {
        lower.lower_func(f,&input.type_names)?;
    }
    // Methods/actors retain their existing unsupported collection gate.
    p.classes.extend(lower.classes);
    Ok(p)
}
