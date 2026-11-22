//! Bounded source-level monomorphization for native .atto Future await adapters.
//!
//! This frontend is deliberately NOT a general generic function ABI. It proves
//! the exact closed-world, pure forwarding body and an explicit consuming
//! call before rewriting one instantiation to the already checked AOT await.
//! No module import, dynamic dispatch, constructor, or task-control authority
//! may be inferred from this transform. Unsupported shapes fail closed.
use crate::diag::{Diagnostic,Pos};
use crate::lexer::{Tok,Token};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Adapter { Future, Actor }
impl Adapter {
    fn name(self)->&'static str {
        match self { Self::Future=>"await_future",Self::Actor=>"await_actor" }
    }
    fn handle(self)->&'static str {
        match self { Self::Future=>"Future",Self::Actor=>"ActorFuture" }
    }
    fn binder(self)->&'static str {
        match self { Self::Future=>"receipt",Self::Actor=>"reply" }
    }
    fn profile(self)->u8 {
        match self { Self::Future=>6,Self::Actor=>7 }
    }
}
fn adapter(t:&[Token],i:usize)->Option<Adapter> {
    if word(t,i,"await_future"){Some(Adapter::Future)}
    else if word(t,i,"await_actor"){Some(Adapter::Actor)}
    else {None}
}
fn word(t:&[Token],i:usize,name:&str)->bool {
    matches!(t.get(i).map(|x|&x.tok),Some(Tok::Ident(s)) if s==name)
}
fn sym(t:&[Token],i:usize,needle:&Tok)->bool {
    t.get(i).is_some_and(|x|&x.tok==needle)
}
fn at(t:&[Token],i:usize)->Pos {
    t.get(i).map_or(Pos{line:1,col:1},|x|x.pos)
}
fn err(t:&[Token],i:usize,code:&'static str,message:&str)->Diagnostic {
    Diagnostic::new(at(t,i),code,message)
}
fn attached(t:&[Token],i:usize)->bool {
    if !word(t,i,"take")||!sym(t,i+1,&Tok::Bang) {return false;}
    let a=at(t,i); let b=at(t,i+1);
    a.line==b.line && a.col.checked_add(4)==Some(b.col)
}
fn signature_end(t:&[Token],start:usize,kind:Adapter)->Result<usize,Diagnostic>{
    // The body is verified token-for-token, not source-regex matched. Even a
    // harmless-looking extra statement or altered payload is not inlined.
    // The generic type T has one known substitution in this ABI: int.
    let w=|s:&str|Tok::Ident(s.to_owned());
    let expected=[
        w("pub"),w("susp"),w("fnc"),w(kind.name()),Tok::Lt,w("T"),Tok::Gt,
        Tok::LParen,w("take"),Tok::Bang,w(kind.handle()),Tok::Lt,w("T"),
        Tok::Gt,w(kind.binder()),Tok::RParen,Tok::Arrow,w("T"),Tok::LBrace,
        w("return"),w("await"),w(kind.binder()),Tok::Semi,Tok::RBrace
    ];
    for (index,token) in expected.iter().enumerate(){
        if !sym(t,start+index,token){
            return Err(err(t,start+index,"E_GENERIC_SIGNATURE",
                "native Future adapter requires exact susp<T>(take! Handle<T>) -> T identity-await body"));
        }
    }
    if !attached(t,start+8) {
        return Err(err(t,start+8,"E_TAKE_LEXICAL",
            "native Future formal requires attached take! rather than take !"));
    }
    Ok(start+expected.len())
}
/// Admit the exact native .atto generic await adapter and specialize T=int.
///
/// No exported symbol, user-defined custom body, heap representation or
/// imported module can masquerade as a compiled generic callable.
pub fn lower(t:Vec<Token>,profile:u8)->Result<Vec<Token>,Diagnostic>{
    let mut removed=vec![false;t.len()];
    let mut declared=[false;2];
    let mut depth=0usize;
    let mut i=0usize;
    while i<t.len(){
        if depth==0 && word(&t,i,"pub") && word(&t,i+1,"susp")
            && word(&t,i+2,"fnc") {
            if let Some(kind)=adapter(&t,i+3){
                if profile!=kind.profile(){
                    return Err(err(&t,i,"E_GENERIC_PROFILE",
                        "generic await adapter needs its matching exact typed AOT backend"));
                }
                let idx=if kind==Adapter::Future{0}else{1};
                if declared[idx]{
                    return Err(err(&t,i,"E_DUPLICATE",
                        "native generic Future adapter declared more than once"));
                }
                let end=signature_end(&t,i,kind)?;
                for removed_token in &mut removed[i..end]{*removed_token=true;}
                declared[idx]=true;
                i=end;
                continue;
            }
        }
        match &t[i].tok {
            Tok::LBrace=>depth+=1,
            Tok::RBrace=>depth=depth.saturating_sub(1),
            _=>{}
        }
        i+=1;
    }
    // This bounded profile is single translation-unit only. A helper inside
    // a `define class/actor/module` (end-delimited) must never be erased
    // as if it were a free function. The normal compiler still owns parsing
    // these declarations; avoid guessing their nested scope boundaries.
    if declared.iter().any(|present|*present)
        && (0..t.len()).any(|at|word(&t,at,"define") || word(&t,at,"import")){
        return Err(err(&t,0,"E_GENERIC_SCOPE",
            "native generic Future adapter requires an independent free function; class, module and imported scopes are not linked"));
    }
    let mut out=Vec::with_capacity(t.len());
    let mut i=0usize;
    while i<t.len(){
        if removed[i]{i+=1;continue;}
        if word(&t,i,"await") {
            if let Some(kind)=adapter(&t,i+1){
                let idx=if kind==Adapter::Future{0}else{1};
                if !declared[idx]{
                    return Err(err(&t,i+1,"E_GENERIC_UNAVAILABLE",
                        "generic Future adapter is not declared in this translation unit; module imports are not yet linked"));
                }
                if profile!=kind.profile(){
                    return Err(err(&t,i+1,"E_GENERIC_PROFILE",
                        "cannot cross a Future/ActorFuture AOT authority profile"));
                }
                if !sym(&t,i+2,&Tok::Lt) || !word(&t,i+3,"int")
                    || !sym(&t,i+4,&Tok::Gt)
                    || !sym(&t,i+5,&Tok::LParen)
                    || !word(&t,i+6,"take")
                    || !sym(&t,i+7,&Tok::Bang)
                    || !matches!(t.get(i+8).map(|x|&x.tok),Some(Tok::Ident(_)))
                    || !sym(&t,i+9,&Tok::RParen){
                    return Err(err(&t,i+1,"E_GENERIC_INSTANTIATION",
                        "only await adapter<int>(take! owned_name) is AOT-verified"));
                }
                if !attached(&t,i+6){
                    return Err(err(&t,i+6,"E_TAKE_LEXICAL",
                        "generic await call requires an attached take!"));
                }
                // Emit the same ownership effect in the typed AOT IR path:
                // await take! receipt.  The existing Future/ActorFuture gate
                // checks actual provenance, linear use and continuation ABI.
                out.push(t[i].clone());
                out.push(t[i+6].clone());
                out.push(t[i+7].clone());
                out.push(t[i+8].clone());
                i+=10;
                continue;
            }
        }
        if adapter(&t,i).is_some(){
            return Err(err(&t,i,"E_GENERIC_CALL",
                "native generic await adapter requires explicit await, <int> and take! owned_name"));
        }
        out.push(t[i].clone());
        i+=1;
    }
    Ok(out)
}
