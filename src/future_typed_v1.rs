//! Strict opt-in typed Future<int> source gate for the portable v5 AOT ABI.
//! This is a compile-time verified, single-use source lowering, NOT a generic
//! Future<T> runtime or a guest-side interpreter. Only closed, straight-line
//! immutable pending futures are supported. Unsupported cases fail closed.
use std::collections::{BTreeMap,BTreeSet};
use crate::ast::{BindKind,Expr,ExprKind,Program,Stmt,Type};
use crate::diag::{Diagnostic,Pos};

fn unsupported(pos:Pos,message:impl Into<String>)->Diagnostic{
    Diagnostic::new(pos,"E_UNSUPPORTED",message)
}
fn type_error(pos:Pos,message:impl Into<String>)->Diagnostic{
    Diagnostic::new(pos,"E_TYPE",message)
}
fn key(expr:&Expr,scalar:&BTreeSet<String>,futures:&BTreeMap<String,bool>)->Result<Expr,Diagnostic>{
    match &expr.kind{
        ExprKind::Int(Some(_))=>Ok(expr.clone()),
        ExprKind::Var(n) if scalar.contains(n)&&!futures.contains_key(n)=>Ok(expr.clone()),
        _=>Err(type_error(expr.pos,
            "pending_future_int(key) requires a literal int or an earlier initialized int scalar local")),
    }
}
fn take_future(expr:&Expr,scalar:&BTreeSet<String>,futures:&mut BTreeMap<String,bool>)
    ->Result<Expr,Diagnostic>{
    match &expr.kind{
        ExprKind::Var(n) if futures.contains_key(n)=>{
            if *futures.get(n).unwrap() {
                return Err(Diagnostic::new(expr.pos,"E_MOVED",
                    format!("Future<int> '{n}' has already been consumed by await")));
            }
            futures.insert(n.clone(),true);
            Ok(expr.clone())
        },
        ExprKind::Call(n,args) if n=="pending_future_int"&&args.len()==1=>
            key(&args[0],scalar,futures),
        _=>Err(type_error(expr.pos,
            "await requires an owned Future<int> from pending_future_int(key), not a bare scalar")),
    }
}
fn await_future(expr:&mut Expr,scalar:&BTreeSet<String>,
    futures:&mut BTreeMap<String,bool>)->Result<(),Diagnostic>{
    let ExprKind::Await(inner)=&mut expr.kind else{
        return Err(type_error(expr.pos,"typed pending-future statement must explicitly await a Future<int>"));
    };
    **inner=take_future(inner,scalar,futures)?;
    Ok(())
}

/// Validate source-level Future<int> construction, linearity, no unconsumed
/// futures, and exact statement/await shape BEFORE ordinary type-checking.
/// Replace only checked owned futures with opaque scalar keys. The shared
/// Rust checker then proves integer initializer and await effects; the
/// existing Wasm-v5 compiler proves every emitted continuation segment.
pub fn lower(p:&mut Program)->Result<(),Diagnostic>{
    if p.funcs.len()!=1 || !p.classes.is_empty() || !p.enums.is_empty() ||
       !p.actors.is_empty() || p.actor_interface.is_some() {
        return Err(unsupported(Pos{line:1,col:1},
            "typed pending-future AOT requires one closed-world main function without classes/actors"));
    }
    let f=&mut p.funcs[0];
    if f.name!="main" || !f.suspends || f.is_generator || f.is_pure ||
       !f.params.is_empty() || f.ret!=Type::Int || f.body.is_empty() ||
       f.body.len()>32 {
        return Err(unsupported(f.pos,
            "typed pending-future AOT requires susp main():int with 1–8 awaits and an explicit final return await"));
    }
    let mut scalar=BTreeSet::<String>::new();
    let mut futures=BTreeMap::<String,bool>::new();
    let mut awaits=0usize;
    let len=f.body.len();
    for (i,stmt) in f.body.iter_mut().enumerate(){
        match stmt {
            Stmt::Bind{kind,name,ty,init,pos} if i+1<len => {
                if scalar.contains(name)||futures.contains_key(name){
                    return Err(Diagnostic::new(*pos,"E_DUPLICATE",
                        format!("duplicate future/scalar local '{name}'")));
                }
                if *ty==Some(Type::FutureInt){
                    if *kind!=BindKind::Val{
                        return Err(type_error(*pos,
                            "owned Future<int> must be declared val; mutable/const future handles are not supported"));
                    }
                    let ExprKind::Call(call,args)=&init.kind else{
                        return Err(type_error(init.pos,"Future<int> initializer must call pending_future_int(key)"));
                    };
                    if call!="pending_future_int" || args.len()!=1 {
                        return Err(type_error(init.pos,"Future<int> initializer must call pending_future_int(key)"));
                    }
                    *init=key(&args[0],&scalar,&futures)?;
                    *ty=Some(Type::Int);
                    futures.insert(name.clone(),false);
                }else{
                    // No inferred Future<int> hiding in a scalar local.
                    // Only simple int scalar assignments are admitted.
                    if !matches!(ty,None|Some(Type::Int)) {
                        return Err(type_error(*pos,"typed pending-future profile only allows int or Future<int> locals"));
                    }
                    match &init.kind {
                        ExprKind::Int(Some(_))=>{},
                        ExprKind::Var(n) if scalar.contains(n)&&!futures.contains_key(n)=>{},
                        _=>return Err(type_error(init.pos,
                            "scalar local must be initialized from an int literal or existing scalar local")),
                    }
                    scalar.insert(name.clone());
                }
            }
            Stmt::Expr(expr) if i+1<len => {
                await_future(expr,&scalar,&mut futures)?;
                awaits+=1;
            }
            Stmt::Return(Some(expr),_) if i+1==len => {
                await_future(expr,&scalar,&mut futures)?;
                awaits+=1;
            }
            _=>return Err(unsupported(f.pos,
                "typed Future<int> AOT supports only straight-line val bindings and awaited statements ending in return await")),
        }
        if futures.len()+scalar.len()>8 || awaits>8 {
            return Err(unsupported(f.pos,"typed future frame exceeds 8 locals or 8 awaits"));
        }
    }
    if awaits==0 || futures.values().any(|consumed|!*consumed){
        return Err(type_error(f.pos,
            "every owned Future<int> must be consumed by exactly one await before main returns"));
    }
    Ok(())
}

/// Add a static source-contract marker alongside the existing guest-v5 ABI
/// marker. An unchanged v5 host can execute this artifact, but the marker
/// lets a verifier distinguish checked Future<int> source from legacy i64 keys.
pub fn mark(mut wasm:Vec<u8>)->Vec<u8>{
    const MARKER:&str="atto.browser.source.future.v1";
    let mut payload=Vec::new();
    uleb(&mut payload,MARKER.len() as u64);
    payload.extend_from_slice(MARKER.as_bytes());
    payload.push(1);
    wasm.push(0); // custom WebAssembly section
    uleb(&mut wasm,payload.len() as u64);
    wasm.extend(payload);
    wasm
}
fn uleb(out:&mut Vec<u8>,mut n:u64){
    loop{
        let mut byte=(n&0x7f) as u8;
        n>>=7;
        if n!=0{byte|=0x80;}
        out.push(byte);
        if n==0 {break;}
    }
}
