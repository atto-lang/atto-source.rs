//! Non-executable scoped Record field preflight.
//! Never turns an indexed Record/Map expression into a guest allocation/read.
use std::collections::BTreeMap;
use crate::ast::{BinOp, Expr, ExprKind, Program, Stmt, Param, UnOp};
use crate::collection_types::{annotation, inferred, matches_expected, static_record_field_type,
    LiteralType as Ty, MAX_LITERAL_DEPTH};
use crate::diag::{Diagnostic, Pos};

type R<T> = Result<T, Diagnostic>;
// Ordinary scalar expression chains have a larger bounded AST budget than
// recursively nested collection literals. collection_types::inferred still
// enforces MAX_LITERAL_DEPTH for actual collection nesting.
const MAX_SCOPE_EXPR_DEPTH: usize = MAX_LITERAL_DEPTH;
const MAX_SCOPE_OP_LINKS: usize = 1024;
fn err(at: Pos, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(at, code, msg)
}
// Prove that the retained fields can be duplicated without manufacturing
// aliases of unique mutable storage. This conservative frontend profile
// admits only immutable scalars, tuples and recursively scalar Records.
fn retain_safe(ty: &Ty, depth: usize) -> bool {
    if depth >= MAX_LITERAL_DEPTH { return false; }
    match ty {
        Ty::Int | Ty::Bool | Ty::Float | Ty::String => true,
        Ty::Record(fields) => fields.iter().all(|(_, t)| retain_safe(t, depth + 1)),
        Ty::Tuple(items) => items.iter().all(|t| retain_safe(t, depth + 1)),
        _ => false,
    }
}
fn is_collection(ty: &Ty) -> bool {
    matches!(ty, Ty::Record(_) | Ty::OpenRecord | Ty::Map(_,_) |
        Ty::Array(_) | Ty::List(_) | Ty::Tuple(_) | Ty::AnyTuple)
}
#[derive(Clone)]
struct Local { actual: Option<Ty>, annotated: Option<Ty> }
#[derive(Clone)]
struct Scope<'a> { program: &'a Program, frames: Vec<BTreeMap<String, Local>> }

impl<'a> Scope<'a> {
    fn new(program: &'a Program) -> Self {
        Self { program, frames: vec![BTreeMap::new()] }
    }
    fn local(&self, name: &str) -> Option<&Local> {
        self.frames.iter().rev().find_map(|frame| frame.get(name))
    }
    fn nested(&mut self, statements: &[Stmt]) -> R<()> {
        self.frames.push(BTreeMap::new());
        let result = self.walk(statements);
        self.frames.pop();
        result
    }
    fn assign(&mut self, name: &str, actual: Option<Ty>, at: Pos) -> R<()> {
        for frame in self.frames.iter_mut().rev() {
            if let Some(local) = frame.get_mut(name) {
                if let Some(ref ty) = actual { matches_expected(ty, local.annotated.as_ref(), at)?; }
                local.actual = actual;
                return Ok(());
            }
        }
        // Unbound variables remain the executable checker's responsibility.
        Ok(())
    }
    fn join(&mut self, alternatives: &[Self]) {
        for (index, frame) in self.frames.iter_mut().enumerate() {
            for (name, local) in frame {
                let values: Vec<Option<Ty>> = alternatives.iter().map(|a|
                    a.frames[index].get(name).and_then(|v| v.actual.clone())).collect();
                if values.iter().all(|v| *v == values[0]) {
                    local.actual = values[0].clone();
                } else if local.annotated == Some(Ty::OpenRecord) ||
                    values.iter().any(|v| matches!(v, Some(Ty::Record(_) | Ty::OpenRecord))) {
                    local.actual = Some(Ty::OpenRecord);
                } else {
                    // An uncertain branch may never justify a static field offset.
                    local.actual = None;
                }
            }
        }
    }
    fn expression(&self, e: &Expr, expected: Option<&Ty>, depth: usize) -> R<Option<Ty>> {
        if depth >= MAX_SCOPE_EXPR_DEPTH {
            return Err(err(e.pos,"E_NESTING","scoped expression nesting exceeded"));
        }
        let ty = match &e.kind {
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Float(_) |
            ExprKind::Str(_) => Some(inferred(e, None, depth)?.ty),
            ExprKind::Var(name) => self.local(name).and_then(|v| v.actual.clone()),
            ExprKind::With(base, updates) => {
                let original = match self.expression(base,None,depth + 1)? {
                    Some(Ty::Record(fields)) => fields,
                    Some(Ty::OpenRecord) => return Err(err(e.pos,"E_WITH_OPEN_RECORD",
                        "with needs a proven closed Record shape; narrow open Record first")),
                    Some(_) => return Err(err(e.pos,"E_WITH_TYPE",
                        "with requires a closed structural Record in this compiler profile")),
                    None => return Err(err(e.pos,"E_WITH_UNRESOLVED",
                        "with receiver type cannot be proved statically")),
                };
                let mut fields: BTreeMap<String, Ty> = original.iter().cloned().collect();
                let mut seen = std::collections::BTreeSet::new();
                for (name, update) in updates {
                    if !seen.insert(name.clone()) {
                        return Err(err(update.pos,"E_DUPLICATE",
                            format!("duplicate with update field '{name}'")));
                    }
                    let updated = self.expression(update,None,depth + 1)?
                        .ok_or_else(|| err(update.pos,"E_WITH_TYPE",
                            format!("with field '{name}' requires a statically known type")))?;
                    if !retain_safe(&updated, depth + 1) {
                        return Err(err(update.pos,"E_WITH_OWNERSHIP",
                            "with cannot implicitly share a mutable or unresolved updated field"));
                    }
                    fields.insert(name.clone(),updated);
                }
                for (name, retained) in &original {
                    if !seen.contains(name) && !retain_safe(retained,depth + 1) {
                        return Err(err(e.pos,"E_WITH_OWNERSHIP",
                            format!("retained field '{name}' lacks verified Copy/share ownership")));
                    }
                }
                let result = Ty::Record(fields.into_iter().collect());
                let _checked_static_layout = crate::layout_plan::record(&result,e.pos)?;
                Some(result)
            },
            ExprKind::Member(base, field) => {
                match self.expression(base,None,depth+1)? {
                    Some(t @ (Ty::Record(_) | Ty::OpenRecord)) =>
                        Some(static_record_field_type(&t,field,e.pos)?),
                    Some(Ty::Map(_, _)) => return Err(err(e.pos,"E_TYPE",
                        "Map keys cannot be accessed as Record dot-fields")),
                    _ => None,
                }
            }
            ExprKind::Index(base,key) => {
                match self.expression(base,None,depth+1)? {
                    Some(t @ (Ty::Record(_) | Ty::OpenRecord)) => {
                        let ExprKind::Str(field) = &key.kind else {
                            return Err(err(key.pos,"E_RECORD_KEY",
                                "Record bracket lookup needs a compile-time string field name"));
                        };
                        Some(static_record_field_type(&t,field,e.pos)?)
                    }
                    _ => {
                        self.expression(key,None,depth+1)?;
                        None // Map/Array indexing remains guest-gated.
                    }
                }
            }
            ExprKind::Record(_) | ExprKind::Map(_) | ExprKind::Array(_) |
            ExprKind::EmptyBraces | ExprKind::Tuple(_) => {
                let ty=inferred(e,expected,depth)?.ty;
                if matches!(ty,Ty::Record(_)) {
                    let _plan=crate::layout_plan::record(&ty,e.pos)?;
                }
                Some(ty)
            }
            // The parser represents the signed minimum as unary minus over
            // the special i64::MAX+1 token. The inner token is intentionally
            // invalid on its own; typecheck the complete negative literal.
            ExprKind::Unary(UnOp::Neg,x) if matches!(&x.kind,ExprKind::Int(None)) =>
                Some(Ty::Int),
            ExprKind::Unary(op,x) => {
                let inner=self.expression(x,None,depth+1)?;
                match (op,inner) {
                    (UnOp::Neg | UnOp::Plus,Some(Ty::Int)) => {
                        if matches!(&x.kind, ExprKind::Int(_)) {
                            Some(inferred(e,None,depth)?.ty)
                        } else { Some(Ty::Int) }
                    },
                    (UnOp::Neg | UnOp::Plus,Some(Ty::Float)) => Some(Ty::Float),
                    (UnOp::Not,Some(Ty::Bool)) => Some(Ty::Bool),
                    _ => None,
                }
            }
            ExprKind::Ownership(_,_,x) | ExprKind::Trap(x) |
            ExprKind::Propagate(x) => { self.expression(x,None,depth+1)?; None }
            ExprKind::Binary(_,_,_) => {
                // Operator chains are left-associated and may be much longer
                // than a nested collection literal. An explicit worklist
                // preserves the parser's 250+ scalar-chain acceptance without
                // growing the Rust checker stack or relaxing literal bounds.
                let mut links=Vec::new();
                let mut current=e;
                while let ExprKind::Binary(op,left,right)=&current.kind {
                    if links.len()>=MAX_SCOPE_OP_LINKS {
                        return Err(err(current.pos,"E_NESTING",
                            "scoped operator chain exceeds the checked link budget"));
                    }
                    links.push((*op,right.as_ref()));
                    current=left.as_ref();
                }
                let mut result=self.expression(current,None,depth+1)?;
                for (op,rhs) in links.into_iter().rev() {
                    let right=self.expression(rhs,None,depth+1)?;
                    result=match (op,result,right) {
                        (BinOp::Add | BinOp::Sub | BinOp::Mul,Some(Ty::Int),Some(Ty::Int)) =>
                            Some(Ty::Int),
                        (BinOp::Add | BinOp::Sub | BinOp::Mul,
                            Some(Ty::Int | Ty::Float),Some(Ty::Float)) |
                        (BinOp::Add | BinOp::Sub | BinOp::Mul,
                            Some(Ty::Float),Some(Ty::Int)) => Some(Ty::Float),
                        (BinOp::Add,Some(Ty::String),Some(Ty::String)) => Some(Ty::String),
                        (BinOp::Eq | BinOp::Ne,Some(Ty::Int),Some(Ty::Int)) |
                        (BinOp::Eq | BinOp::Ne,Some(Ty::Bool),Some(Ty::Bool)) |
                        (BinOp::Eq | BinOp::Ne,Some(Ty::String),Some(Ty::String)) => Some(Ty::Bool),
                        (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
                            Some(Ty::Int),Some(Ty::Int)) => Some(Ty::Bool),
                        _ => None,
                    };
                }
                result
            }
            ExprKind::Ternary(c,a,b) => {
                self.expression(c,None,depth+1)?;
                let left=self.expression(a,expected,depth+1)?;
                let right=self.expression(b,expected,depth+1)?;
                if left==right { left }
                else if matches!((&left,&right),
                    (Some(Ty::Record(_)),Some(Ty::Record(_))) |
                    (Some(Ty::OpenRecord),Some(Ty::Record(_))) |
                    (Some(Ty::Record(_)),Some(Ty::OpenRecord))) {
                    Some(Ty::OpenRecord)
                } else { None }
            }
            // Browser's existing And/Or/Xor bool-pack ABI accepts an array
            // spelling as a *call argument* only. It never allocates a general
            // Array value. Visit item expressions without asking the collection
            // literal inference pass to invent a runtime array ABI. The normal
            // typed call checker still validates booleans and rejects effects.
            ExprKind::Call(name,args) if matches!(name.as_str(),"And"|"Or"|"Xor") => {
                for arg in args {
                    if let ExprKind::Array(items)=&arg.kind {
                        for item in items {
                            self.expression(item,Some(&Ty::Bool),depth+1)?;
                        }
                    } else { self.expression(arg,None,depth+1)?; }
                }
                None
            }
            ExprKind::Call(_,args) | ExprKind::New(_,args) => {
                for arg in args {self.expression(arg,None,depth+1)?;}
                None
            }
            ExprKind::MethodCall(recv,_,args) => {
                self.expression(recv,None,depth+1)?;
                for arg in args {self.expression(arg,None,depth+1)?;}
                None
            }
            // Scalars, regular class members and lambdas still belong to the
            // full executable checker; this pass must not invent their types.
            _ => None,
        };
        if let Some(ref actual) = ty {matches_expected(actual,expected,e.pos)?;}
        Ok(ty)
    }
    fn walk(&mut self, body: &[Stmt]) -> R<()> {
        for statement in body {
            match statement {
                Stmt::Bind {name,ty,init,pos,..} => {
                    let annotated=ty.map(|t|annotation(t,self.program,*pos))
                        .transpose()?.flatten();
                    let actual=self.expression(init,annotated.as_ref(),0)?;
                    if annotated.as_ref().is_some_and(is_collection) && actual.is_none() {
                        return Err(err(init.pos,"E_UNSUPPORTED",
                            "collection binding needs a statically known value"));
                    }
                    self.frames.last_mut().unwrap().insert(name.clone(),Local {actual,annotated});
                }
                Stmt::Assign {name,value,pos} => {
                    let ty=self.expression(value,None,0)?;
                    self.assign(name,ty,*pos)?;
                }
                Stmt::Expr(e) | Stmt::Throw(e,_) | Stmt::Raise(e,_) |
                Stmt::Yield(e,_) | Stmt::YieldFrom(e,_) => {self.expression(e,None,0)?;}
                Stmt::Return(Some(e),_) => {self.expression(e,None,0)?;}
                Stmt::FieldAssign {target,value,..} => {
                    self.expression(target,None,0)?;
                    self.expression(value,None,0)?;
                }
                Stmt::Block(inner) | Stmt::StateWrite {body:inner,..} => self.nested(inner)?,
                Stmt::If {arms,otherwise,..} => {
                    let starting=self.clone();
                    let mut alternatives=Vec::new();
                    for (cond,body) in arms {
                        starting.expression(cond,None,0)?;
                        let mut branch=starting.clone();
                        branch.nested(body)?;
                        alternatives.push(branch);
                    }
                    let mut default=starting;
                    if let Some(body)=otherwise {default.nested(body)?;}
                    alternatives.push(default);
                    self.join(&alternatives);
                }
                Stmt::While {cond,body,..} => {
                    self.expression(cond,None,0)?;
                    let mut iter=self.clone();
                    iter.nested(body)?;
                    let before=self.clone();
                    self.join(&[before,iter]);
                }
                Stmt::Loop {body,..} => {
                    let mut iter=self.clone();
                    iter.nested(body)?;
                    let before=self.clone();
                    self.join(&[before,iter]);
                }
                Stmt::For {init,cond,update,body,..} => {
                    let mut iter=self.clone();
                    iter.frames.push(BTreeMap::new());
                    if let Some(s)=init {iter.walk(std::slice::from_ref(s.as_ref()))?;}
                    if let Some(e)=cond {iter.expression(e,None,0)?;}
                    iter.nested(body)?;
                    if let Some(s)=update {iter.walk(std::slice::from_ref(s.as_ref()))?;}
                    iter.frames.pop();
                    let before=self.clone();
                    self.join(&[before,iter]);
                }
                Stmt::ForIn {name,source,body,..} => {
                    self.expression(source,None,0)?;
                    let mut iter=self.clone();
                    iter.frames.push(BTreeMap::new());
                    iter.frames.last_mut().unwrap().insert(name.clone(),
                        Local {actual:None,annotated:None});
                    iter.nested(body)?;
                    iter.frames.pop();
                    let before=self.clone();
                    self.join(&[before,iter]);
                }
                Stmt::Match {subject,arms,..} => {
                    self.expression(subject,None,0)?;
                    let mut alternatives=vec![self.clone()];
                    for arm in arms {
                        let mut branch=self.clone();
                        branch.nested(&arm.body)?;
                        alternatives.push(branch);
                    }
                    self.join(&alternatives);
                }
                Stmt::Try {body,catch,finally,..} => {
                    let mut alternatives=vec![self.clone()];
                    let mut success=self.clone();
                    success.nested(body)?;
                    alternatives.push(success);
                    if let Some((name,body))=catch {
                        let mut handler=self.clone();
                        handler.frames.push(BTreeMap::new());
                        handler.frames.last_mut().unwrap().insert(name.clone(),
                            Local {actual:None,annotated:None});
                        handler.walk(body)?;
                        handler.frames.pop();
                        alternatives.push(handler);
                    }
                    self.join(&alternatives);
                    if let Some(body)=finally {self.nested(body)?;}
                }
                Stmt::Defer {body,..} => {
                    let mut check=self.clone();
                    check.nested(body)?;
                }
                Stmt::Recover {params,err,cursor,args,body,..} => {
                    for e in args {self.expression(e,None,0)?;}
                    let mut check=self.clone();
                    check.frames.push(BTreeMap::new());
                    for (name,_,_) in params {
                        check.frames.last_mut().unwrap().insert(name.clone(),
                            Local {actual:None,annotated:None});
                    }
                    for name in [err,cursor] {
                        check.frames.last_mut().unwrap().insert(name.clone(),
                            Local {actual:None,annotated:None});
                    }
                    check.walk(body)?;
                }
                Stmt::Return(None,_) | Stmt::Break(_) | Stmt::Continue(_) => {}
            }
        }
        Ok(())
    }
}
fn routine(program: &Program, params: &[Param], body: &[Stmt]) -> R<()> {
    let mut scopes=Scope::new(program);
    for param in params {
        let annotated=annotation(param.ty,program,param.pos)?;
        scopes.frames[0].insert(param.name.clone(),
            Local {actual:annotated.clone(),annotated});
    }
    scopes.walk(body)
}
/// Checked field-access syntax, not guest allocation or ABI validation.
pub fn analyze(program: &Program) -> R<()> {
    for func in &program.funcs {routine(program,&func.params,&func.body)?;}
    for class in &program.classes {
        if let Some(ctor)=&class.ctor {routine(program,&ctor.params,&ctor.body)?;}
        for method in &class.methods {routine(program,&method.params,&method.body)?;}
    }
    for actor in &program.actors {
        for method in &actor.methods {routine(program,&method.params,&method.body)?;}
        if let Some(recv)=&actor.receive {routine(program,&[],&recv.body)?;}
    }
    Ok(())
}
