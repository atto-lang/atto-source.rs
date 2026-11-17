//! Non-executable scoped Record field preflight.
//! Never turns an indexed Record/Map expression into a guest allocation/read.
use std::collections::BTreeMap;
use crate::ast::{Expr, ExprKind, Program, Stmt, Param};
use crate::collection_types::{annotation, inferred, matches_expected, static_record_field_type,
    LiteralType as Ty, MAX_LITERAL_DEPTH};
use crate::diag::{Diagnostic, Pos};

type R<T> = Result<T, Diagnostic>;
fn err(at: Pos, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(at, code, msg)
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
        if depth >= MAX_LITERAL_DEPTH {
            return Err(err(e.pos,"E_NESTING","scoped type expression nesting exceeded"));
        }
        let ty = match &e.kind {
            ExprKind::Var(name) => self.local(name).and_then(|v| v.actual.clone()),
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
            ExprKind::Ownership(_,_,x) | ExprKind::Unary(_,x) | ExprKind::Trap(x) |
            ExprKind::Propagate(x) => { self.expression(x,None,depth+1)?; None }
            ExprKind::Binary(_,a,b) => {
                self.expression(a,None,depth+1)?;
                self.expression(b,None,depth+1)?;
                None
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
                    let annotated=ty.map(|t|annotation(*t,self.program,*pos))
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
