//! Opt-in, typed, owned pending Future continuation profile.
//! Static Wasm quanta, one invocation per instance, no guest imports,
//! allocation, interpreter, JSPI, or memory/table export. Not the legacy
//! v5 success-only task ABI (#197); this ABI carries failure receipts.
use crate::diag::{Diagnostic,Pos};
use crate::lexer::{lex,Tok,Token};
use super::{uleb,sleb,name,section};
use std::collections::{BTreeMap,BTreeSet};

const I32:u8=0x7f;
const I64:u8=0x7e;
const MAX:usize=8;
#[derive(Clone,Copy)]
enum Stage { Body(u32), Cleanup(u32), Recover(u32) }
impl Stage { fn slot(self)->u32 { match self {Self::Body(n)|Self::Cleanup(n)|Self::Recover(n)=>n} } }
struct Plan { stages:Vec<Stage>, literal:i64 }
struct Parser { t:Vec<Token>, at:usize, formals:BTreeMap<String,u32>, consumed:BTreeSet<u32> }
impl Parser {
    fn pos(&self)->Pos { self.t.get(self.at).map_or(Pos{line:1,col:1},|t|t.pos) }
    fn err(&self,code:&'static str,msg:&str)->Diagnostic { Diagnostic::new(self.pos(),code,msg) }
    fn peek(&self)->&Tok { &self.t[self.at].tok }
    fn eat(&mut self,t:Tok)->bool { if *self.peek()==t { self.at+=1;true } else {false} }
    fn need(&mut self,t:Tok)->Result<(),Diagnostic> {
        if self.eat(t.clone()) {Ok(())} else {Err(self.err("E_PENDING_UNWIND_PROFILE",&format!("expected {t:?}")))}
    }
    fn word(&mut self,w:&str)->Result<(),Diagnostic> { self.need(Tok::Ident(w.into())) }
    fn ident(&mut self)->Result<String,Diagnostic> {
        if let Tok::Ident(s)=self.peek() {
            if !s.is_empty() && s.bytes().enumerate().all(|(i,b)|b==b'_'||b.is_ascii_alphabetic()||(i>0&&b.is_ascii_digit()))
                && !["pub","fnc","run","suspends","Future","int","defer","recover","return","await","take","move","borrow","raise","throw","panic","let","val","const","true","false","if","self","new","select","mut"].contains(&s.as_str()) {
                let s=s.clone();self.at+=1;return Ok(s)
            }
        }
        Err(self.err("E_PENDING_UNWIND_PROFILE","expected nonreserved ASCII binder"))
    }
    fn consume(&mut self)->Result<u32,Diagnostic> {
        let n=self.ident()?;
        let slot=*self.formals.get(&n).ok_or_else(||self.err("E_OWNERSHIP","await requires an owned Future formal"))?;
        if !self.consumed.insert(slot){return Err(self.err("E_MOVED","Future formal used twice"))}
        Ok(slot)
    }
    fn number(&mut self)->Result<i64,Diagnostic>{
        let neg=self.eat(Tok::Minus);
        if let Tok::Int(Some(n))=*self.peek(){
            if n<=i64::MAX as u64+u64::from(neg){
                self.at+=1;
                return Ok(if neg { if n==1u64<<63{i64::MIN}else{-(n as i64)} } else {n as i64});
            }
        }
        Err(self.err("E_INTEGER_RANGE","signed i64 literal required"))
    }
    fn parse(mut self)->Result<Plan,Diagnostic>{
        self.word("pub")?;self.word("suspends")?;self.word("fnc")?;self.word("run")?;self.need(Tok::LParen)?;
        loop {
            if self.formals.len()>=MAX{return Err(self.err("E_PENDING_UNWIND_PROFILE","formal quota exceeded"))}
            let p=self.pos();
            if !self.eat(Tok::Ident("take".into())){return Err(self.err("E_OWNERSHIP","sealed Future requires take! formal"))}
            let bang=self.pos();
            if p.line!=bang.line||p.col.checked_add(4)!=Some(bang.col){return Err(self.err("E_OWNERSHIP","take! must be contiguous"))}
            self.need(Tok::Bang)?;self.word("Future")?;self.need(Tok::Lt)?;self.word("int")?;self.need(Tok::Gt)?;
            let n=self.ident()?;let slot=self.formals.len() as u32+1;
            if self.formals.insert(n,slot).is_some(){return Err(self.err("E_DUPLICATE","duplicate Future formal"))}
            if !self.eat(Tok::Comma){break}
        }
        self.need(Tok::RParen)?;self.need(Tok::Colon)?;self.word("int")?;self.need(Tok::LBrace)?;
        let mut defers=Vec::new();let mut recovery=None;
        loop {
            if self.eat(Tok::Ident("defer".into())) {
                self.word("await")?;defers.push(Stage::Cleanup(self.consume()?));self.need(Tok::Semi)?;
            }else if self.eat(Tok::Ident("recover".into())){
                if recovery.is_some(){return Err(self.err("E_PENDING_UNWIND_PROFILE","only one recovery callback"))}
                self.need(Tok::Pipe)?;let err=self.ident()?;self.need(Tok::Comma)?;let cursor=self.ident()?;self.need(Tok::Pipe)?;
                if err==cursor||self.formals.contains_key(&err)||self.formals.contains_key(&cursor){return Err(self.err("E_DUPLICATE","recovery binder collision"))}
                self.need(Tok::Arrow)?;self.need(Tok::LBrace)?;self.word("return")?;self.word("await")?;
                recovery=Some(Stage::Recover(self.consume()?));self.need(Tok::Semi)?;self.need(Tok::RBrace)?;self.need(Tok::Semi)?;
            }else{break}
        }
        self.word("return")?;
        let body=if self.eat(Tok::Ident("await".into())){Some(Stage::Body(self.consume()?))}else{None};
        let literal=if body.is_none(){self.number()?}else{0};
        self.need(Tok::Semi)?;self.need(Tok::RBrace)?;
        if *self.peek()!=Tok::Eof || self.consumed.len()!=self.formals.len() || (recovery.is_some()&&body.is_none()) {
            return Err(self.err("E_PENDING_UNWIND_PROFILE","closed profile requires exactly one lexical use of every owned Future and recovery requires body await"))
        }
        let mut stages=Vec::new();stages.extend(body);stages.extend(defers.into_iter().rev());stages.extend(recovery);
        Ok(Plan{stages,literal})
    }
}
// Private mutable globals. No memory is exported or allocated.
// One instance is permanently retired; no generation recycling.
const OWNER:u32=0;const EPOCH:u32=1;const PC:u32=2;const RANK:u32=3;const RECOVERED:u32=4;
const VALUE:u32=5;const ERROR:u32=6;const COMPLETION_KIND:u32=7;const COMPLETION_VALUE:u32=8;
const SUP_COUNT:u32=9;const SUP_BASE:u32=10;const SUP_RANK_BASE:u32=18;
const READY:i32=16;const DONE:i32=32;const RETIRED:i32=33;
fn op(c:&mut Vec<u8>,b:u8){c.push(b)}
fn ic(c:&mut Vec<u8>,n:i32){op(c,0x41);sleb(c,n as i64)}
fn lc(c:&mut Vec<u8>,n:i64){op(c,0x42);sleb(c,n)}
fn get(c:&mut Vec<u8>,n:u32){op(c,0x23);uleb(c,n as u64)}
fn set(c:&mut Vec<u8>,n:u32){op(c,0x24);uleb(c,n as u64)}
fn local(c:&mut Vec<u8>,n:u32){op(c,0x20);uleb(c,n as u64)}
fn si(c:&mut Vec<u8>,g:u32,n:i32){ic(c,n);set(c,g)}
fn sl(c:&mut Vec<u8>,g:u32,n:i64){lc(c,n);set(c,g)}
fn eq(c:&mut Vec<u8>,g:u32,n:i32){get(c,g);ic(c,n);op(c,0x46)}
fn begin(c:&mut Vec<u8>){c.extend([0x04,0x40])}
fn end(c:&mut Vec<u8>){op(c,0x0b)}
fn ret(c:&mut Vec<u8>,n:i32){ic(c,n);op(c,0x0f)}
fn bad(c:&mut Vec<u8>,trap:bool){begin(c);if trap{op(c,0x00)}else{ret(c,-1)}end(c)}
fn auth(c:&mut Vec<u8>,trap:bool){
    for (i,g) in [(0,OWNER),(1,EPOCH)] {
        local(c,i);ic(c,0);op(c,0x4c);bad(c,trap);
        local(c,i);get(c,g);op(c,0x47);bad(c,trap);
    }
}
fn terminal(c:&mut Vec<u8>){auth(c,true);eq(c,PC,DONE);op(c,0x45);bad(c,true)}
fn suppress(c:&mut Vec<u8>,rank_local:Option<u32>,error_local:Option<u32>){
    // At most eight distinct Future failures; seven suppressed entries fit.
    for i in 0..MAX as u32 {
        eq(c,SUP_COUNT,i as i32);begin(c);
        match error_local {Some(n)=>local(c,n),None=>get(c,ERROR)}set(c,SUP_BASE+i);
        match rank_local {Some(n)=>local(c,n),None=>get(c,RANK)}set(c,SUP_RANK_BASE+i);
        si(c,SUP_COUNT,i as i32+1);op(c,0x0f);end(c);
    }
    op(c,0x00);
}
fn merge(c:&mut Vec<u8>,rank:i32){
    ic(c,rank);get(c,COMPLETION_VALUE);op(c,0x10);uleb(c,0)
}
fn advance(c:&mut Vec<u8>,p:&Plan,next:usize){
    if next>=p.stages.len(){si(c,PC,DONE);ret(c,2);return}
    if matches!(p.stages[next],Stage::Recover(_)){
        // Only original recoverable raise reaches recovery.
        eq(c,RANK,1);op(c,0x45);begin(c);si(c,PC,DONE);ret(c,2);end(c);
    }
    si(c,PC,next as i32+1);ret(c,1)
}
type FnDef=(&'static str,Vec<u8>,Vec<u8>,Vec<u8>);
fn emit(p:&Plan)->Vec<u8>{
    let mut funcs:Vec<FnDef>=Vec::new();
    // Internal merge(rank,error) helper; preserve dominant failure and ordered suppression.
    let mut c=vec![0];
    local(&mut c,0);get(&mut c,RANK);op(&mut c,0x4a);begin(&mut c);
    get(&mut c,RANK);begin(&mut c);op(&mut c,0x10);uleb(&mut c,1);end(&mut c);
    local(&mut c,0);set(&mut c,RANK);local(&mut c,1);set(&mut c,ERROR);
    op(&mut c,0x05);local(&mut c,0);local(&mut c,1);op(&mut c,0x10);uleb(&mut c,2);end(&mut c);end(&mut c);
    funcs.push(("",vec![I32,I64],vec![],c));
    let mut c=vec![0];suppress(&mut c,None,None);end(&mut c);funcs.push(("",vec![],vec![],c));
    let mut c=vec![0];suppress(&mut c,Some(0),Some(1));end(&mut c);funcs.push(("",vec![I32,I64],vec![],c));
    let mut c=vec![0];eq(&mut c,PC,0);op(&mut c,0x45);bad(&mut c,false);
    for i in 0..2 {local(&mut c,i);ic(&mut c,0);op(&mut c,0x4c);bad(&mut c,false);}
    local(&mut c,0);set(&mut c,OWNER);local(&mut c,1);set(&mut c,EPOCH);sl(&mut c,VALUE,p.literal);si(&mut c,PC,1);ic(&mut c,1);end(&mut c);
    funcs.push(("unwind_start_v1",vec![I32,I32],vec![I32],c));
    let mut c=vec![0];auth(&mut c,false);
    for (i,s) in p.stages.iter().enumerate(){eq(&mut c,PC,i as i32+1);begin(&mut c);ret(&mut c,s.slot() as i32);end(&mut c);}
    ic(&mut c,-1);end(&mut c);funcs.push(("unwind_pending_v1",vec![I32,I32],vec![I32],c));
    let mut c=vec![0];auth(&mut c,false);
    local(&mut c,3);ic(&mut c,2);op(&mut c,0x4b);bad(&mut c,false); // unsigned status > 2
    local(&mut c,3);begin(&mut c);local(&mut c,4);lc(&mut c,0);op(&mut c,0x57);bad(&mut c,false);end(&mut c); // error ID > 0
    for (i,s) in p.stages.iter().enumerate(){
        eq(&mut c,PC,i as i32+1);begin(&mut c);
        local(&mut c,2);ic(&mut c,s.slot() as i32);op(&mut c,0x47);bad(&mut c,false);
        local(&mut c,3);set(&mut c,COMPLETION_KIND);local(&mut c,4);set(&mut c,COMPLETION_VALUE);
        si(&mut c,PC,READY+i as i32+1);ret(&mut c,0);end(&mut c);
    }
    ic(&mut c,-1);end(&mut c);funcs.push(("unwind_complete_v1",vec![I32,I32,I32,I32,I64],vec![I32],c));
    let mut c=vec![0];auth(&mut c,false);
    for (i,s) in p.stages.iter().enumerate(){
        eq(&mut c,PC,READY+i as i32+1);begin(&mut c);
        eq(&mut c,COMPLETION_KIND,0);begin(&mut c);
        if matches!(s,Stage::Body(_)|Stage::Recover(_)){get(&mut c,COMPLETION_VALUE);set(&mut c,VALUE);}
        if matches!(s,Stage::Recover(_)){si(&mut c,RECOVERED,1);}
        op(&mut c,0x05);
        eq(&mut c,COMPLETION_KIND,2);begin(&mut c);merge(&mut c,3);
        op(&mut c,0x05);merge(&mut c,if matches!(s,Stage::Body(_)){1}else{2});end(&mut c);end(&mut c);
        si(&mut c,COMPLETION_KIND,0);sl(&mut c,COMPLETION_VALUE,0);
        advance(&mut c,p,i+1);end(&mut c);
    }
    ic(&mut c,-1);end(&mut c);funcs.push(("unwind_resume_v1",vec![I32,I32],vec![I32],c));
    let mut c=vec![0];terminal(&mut c);
    eq(&mut c,RECOVERED,1);begin(&mut c);ret(&mut c,2);end(&mut c);
    eq(&mut c,RANK,0);begin(&mut c);ret(&mut c,1);end(&mut c);
    get(&mut c,RANK);ic(&mut c,2);op(&mut c,0x6a);end(&mut c);
    funcs.push(("unwind_outcome_v1",vec![I32,I32],vec![I32],c));
    let mut c=vec![0];terminal(&mut c);eq(&mut c,RANK,0);eq(&mut c,RECOVERED,1);op(&mut c,0x72);op(&mut c,0x45);bad(&mut c,true);get(&mut c,VALUE);end(&mut c);
    funcs.push(("unwind_result_v1",vec![I32,I32],vec![I64],c));
    for (n,g,t) in [("unwind_error_v1",ERROR,I64),("unwind_suppressed_count_v1",SUP_COUNT,I32)]{
        let mut c=vec![0];terminal(&mut c);get(&mut c,g);end(&mut c);funcs.push((n,vec![I32,I32],vec![t],c));
    }
    for (n,base,t) in [("unwind_suppressed_error_v1",SUP_BASE,I64),("unwind_suppressed_rank_v1",SUP_RANK_BASE,I32)]{
        let mut c=vec![0];terminal(&mut c);local(&mut c,2);get(&mut c,SUP_COUNT);op(&mut c,0x4f);bad(&mut c,true);
        for i in 0..MAX as u32 {local(&mut c,2);ic(&mut c,i as i32);op(&mut c,0x46);begin(&mut c);get(&mut c,base+i);op(&mut c,0x0f);end(&mut c);}
        op(&mut c,0x00);end(&mut c);funcs.push((n,vec![I32,I32,I32],vec![t],c));
    }
    let mut c=vec![0];auth(&mut c,false);eq(&mut c,PC,DONE);op(&mut c,0x45);bad(&mut c,false);
    for g in 0..SUP_RANK_BASE+MAX as u32 {if g==VALUE||g==ERROR||g==COMPLETION_VALUE||(SUP_BASE..SUP_RANK_BASE).contains(&g){sl(&mut c,g,0)}else{si(&mut c,g,0)}}
    si(&mut c,PC,RETIRED);ic(&mut c,0);end(&mut c);funcs.push(("unwind_retire_v1",vec![I32,I32],vec![I32],c));
    let mut out=b"\0asm\x01\0\0\0".to_vec();
    let mut types=Vec::new();uleb(&mut types,funcs.len() as u64);
    let mut indices=Vec::new();uleb(&mut indices,funcs.len() as u64);
    let mut exports=Vec::new();uleb(&mut exports,(funcs.len()-3) as u64);
    let mut code=Vec::new();uleb(&mut code,funcs.len() as u64);
    for (i,(n,args,ret,body)) in funcs.iter().enumerate(){
        op(&mut types,0x60);uleb(&mut types,args.len() as u64);types.extend(args);uleb(&mut types,ret.len() as u64);types.extend(ret);
        uleb(&mut indices,i as u64);
        if !n.is_empty(){name(&mut exports,n);op(&mut exports,0);uleb(&mut exports,i as u64)}
        uleb(&mut code,body.len() as u64);code.extend(body);
    }
    section(&mut out,1,types);section(&mut out,3,indices);
    let mut globals=Vec::new();uleb(&mut globals,(SUP_RANK_BASE+MAX as u32) as u64);
    for g in 0..SUP_RANK_BASE+MAX as u32 {
        let ty=if g==VALUE||g==ERROR||g==COMPLETION_VALUE||(SUP_BASE..SUP_RANK_BASE).contains(&g){I64}else{I32};
        globals.extend([ty,1]);if ty==I64{lc(&mut globals,0)}else{ic(&mut globals,0)}end(&mut globals);
    }
    section(&mut out,6,globals);section(&mut out,7,exports);section(&mut out,10,code);
    let mut marker=Vec::new();name(&mut marker,"atto.browser.owned_unwind.v1");marker.extend([1,p.stages.len() as u8]);section(&mut out,0,marker);out
}
pub(crate) fn compile(source:&str)->Result<Vec<u8>,Diagnostic>{
    if source.len()>crate::SOURCE_CAP{return Err(Diagnostic::new(Pos{line:1,col:1},"E_SOURCE_LIMIT","owned unwind source cap"))}
    let p=Parser{t:lex(source)?,at:0,formals:BTreeMap::new(),consumed:BTreeSet::new()}.parse()?;
    Ok(emit(&p))
}
