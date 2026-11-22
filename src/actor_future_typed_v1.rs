//! Fail-closed one-shot ActorFuture<int> typed frontend.
//! Actor ID/request key are compile-time constants; incarnation is a trusted
//! host binding at task admission, NEVER an unauthenticated source integer.
//! Actor replies use single-use bearer grants and the established v5 AOT ABI.
use std::collections::{BTreeMap,BTreeSet};
use crate::ast::{BindKind,Expr,ExprKind,Program,Stmt,Type};
use crate::diag::{Diagnostic,Pos};

#[derive(Clone,Copy)]
pub struct ActorAwait { pub actor_id:u32,pub key:i64 }
fn err(p:Pos,code:&'static str,msg:impl Into<String>)->Diagnostic {
    Diagnostic::new(p,code,msg)
}
fn posint(e:&Expr,name:&str)->Result<u32,Diagnostic>{
    match e.kind {
        ExprKind::Int(Some(n)) if (1..=i32::MAX as u64).contains(&n)=>Ok(n as u32),
        _=>Err(err(e.pos,"E_ACTOR_FUTURE",
            format!("{name} must be a positive compile-time i32 literal, never a forged ActorRef or actor incarnation"))),
    }
}
fn key(e:&Expr)->Result<i64,Diagnostic>{
    match e.kind {
        ExprKind::Int(Some(n)) if n<=i64::MAX as u64=>Ok(n as i64),
        _=>Err(err(e.pos,"E_ACTOR_FUTURE",
            "actor request key must be a nonnegative compile-time signed-i64 literal")),
    }
}
fn ctor(expr:&Expr)->Result<ActorAwait,Diagnostic> {
    match &expr.kind {
        ExprKind::Call(name,args) if name=="actor_future_int"&&args.len()==2 =>
            Ok(ActorAwait{actor_id:posint(&args[0],"actor ID")?,key:key(&args[1])?}),
        _=>Err(err(expr.pos,"E_ACTOR_FUTURE",
            "ActorFuture<int> requires actor_future_int(actor_id,request_key) with two static integer literals")),
    }
}
fn awaited(expr:&mut Expr,futures:&mut BTreeMap<String,(ActorAwait,bool)>)
    ->Result<ActorAwait,Diagnostic>{
    let ExprKind::Await(inner)=&mut expr.kind else {
        return Err(err(expr.pos,"E_ACTOR_FUTURE","actor reply must be consumed with explicit await"));
    };
    match &inner.kind {
        ExprKind::Var(n) if futures.contains_key(n)=>{
            let (info,consumed)=futures.get_mut(n).unwrap();
            if *consumed {
                return Err(err(inner.pos,"E_MOVED",
                    format!("ActorFuture<int> '{n}' was already consumed by await")));
            }
            *consumed=true;
            Ok(*info)
        }
        ExprKind::Call(name,_) if name=="actor_future_int"=>{
            let info=ctor(inner)?;
            **inner=Expr{kind:ExprKind::Int(Some(info.key as u64)),pos:inner.pos};
            Ok(info)
        }
        _=>Err(err(inner.pos,"E_ACTOR_FUTURE",
            "await requires an owned ActorFuture<int>, not a scalar, Future<int>, or stream")),
    }
}
pub fn lower(p:&mut Program)->Result<Vec<ActorAwait>,Diagnostic>{
    if p.funcs.len()!=1||!p.classes.is_empty()||!p.actors.is_empty()||
       !p.enums.is_empty()||p.actor_interface.is_some(){
        return Err(err(Pos{line:1,col:1},"E_UNSUPPORTED",
            "ActorFuture<int> AOT currently requires one closed-world main and an external actor runtime"));
    }
    let f=&mut p.funcs[0];
    if f.name!="main"||!f.suspends||f.is_generator||f.is_pure||
       !f.params.is_empty()||f.ret!=Type::Int||f.body.is_empty()||
       f.body.len()>32 {
        return Err(err(f.pos,"E_UNSUPPORTED",
            "ActorFuture<int> AOT requires a single susp main():int with explicit return await"));
    }
    let mut scalars=BTreeSet::<String>::new();
    let mut futures=BTreeMap::<String,(ActorAwait,bool)>::new();
    let mut awaits=Vec::new();
    let len=f.body.len();
    for (index,stmt) in f.body.iter_mut().enumerate() {
        match stmt {
            Stmt::Bind{kind,name,ty,init,pos} if index+1<len =>{
                if scalars.contains(name)||futures.contains_key(name){
                    return Err(err(*pos,"E_DUPLICATE",format!("duplicate ActorFuture/local '{name}'")));
                }
                match ty {
                    Some(Type::ActorFutureInt) if *kind==BindKind::Val=>{
                        let desc=ctor(init)?;
                        *init=Expr{kind:ExprKind::Int(Some(desc.key as u64)),pos:init.pos};
                        *ty=Some(Type::Int);
                        futures.insert(name.clone(),(desc,false));
                    }
                    Some(Type::ActorFutureInt)=>return Err(err(*pos,"E_ACTOR_FUTURE",
                        "ActorFuture<int> must be immutable val; mutable/const handles are forbidden")),
                    None|Some(Type::Int)=>{
                        match &init.kind {
                            ExprKind::Int(Some(n)) if *n<=i64::MAX as u64=>{},
                            ExprKind::Var(n) if scalars.contains(n)=>{},
                            _=>return Err(err(init.pos,"E_ACTOR_FUTURE",
                                "scalar local must be an int literal or earlier scalar (ActorFuture cannot alias/escape)")),
                        }
                        scalars.insert(name.clone());
                    }
                    _=>return Err(err(*pos,"E_UNSUPPORTED_TYPE",
                        "actor-future AOT supports only int scalar locals and ActorFuture<int>")),
                }
            }
            Stmt::Expr(expr) if index+1<len=>{
                awaits.push(awaited(expr,&mut futures)?);
            }
            Stmt::Return(Some(expr),_) if index+1==len=>{
                awaits.push(awaited(expr,&mut futures)?);
            }
            _=>return Err(err(f.pos,"E_UNSUPPORTED",
                "ActorFuture AOT supports only straight-line immutable bindings and explicit awaits ending in return await")),
        }
        if futures.len()+scalars.len()>8||awaits.len()>8 {
            return Err(err(f.pos,"E_SOURCE_LIMIT","ActorFuture frame supports at most eight locals and eight awaits"));
        }
    }
    if awaits.is_empty()||futures.values().any(|(_,consumed)|!*consumed){
        return Err(err(f.pos,"E_ACTOR_FUTURE",
            "every ActorFuture<int> must be consumed exactly once before returning"));
    }
    Ok(awaits)
}

/// Deterministic versioned manifest of ACTOR IDs + awaited request keys in
/// the exact static await order. Host validates guest key per step and
/// supplies the ACTIVE producer incarnation from its trusted registry.
pub fn mark(mut wasm:Vec<u8>,entries:&[ActorAwait])->Vec<u8>{
    let name="atto.browser.source.actor_future.v1";
    let mut content=Vec::new();
    uleb(&mut content,name.len() as u64);
    content.extend_from_slice(name.as_bytes());
    content.push(1);
    content.push(entries.len() as u8);
    for entry in entries {
        content.extend_from_slice(&entry.actor_id.to_le_bytes());
        content.extend_from_slice(&entry.key.to_le_bytes());
    }
    wasm.push(0);
    uleb(&mut wasm,content.len() as u64);
    wasm.extend(content);
    wasm
}
fn uleb(out:&mut Vec<u8>,mut value:u64){
    loop{
        let mut b=(value&127) as u8;
        value>>=7;
        if value!=0{b|=128;}
        out.push(b);
        if value==0{return;}
    }
}
