//! Narrow M8 canonical browser compiler path: typed owned-i64 actor safe point.
//! Separate no-import, one-frame-per-Wasm-instance M8 ABI. NOT general actor
//! runtime, host scheduling, critical loans or language await/select.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

const MAX_TOKENS: usize = 512;
pub const MAX_REDUCTIONS: i32 = 65536;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan { pub actor: String, pub initial_total: i64, pub initial_index: i64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op { Entry, Guard, CheckedAddAndIncrement, Cooperate, Done }
pub const IR: [Op; 5] = [Op::Entry, Op::Guard, Op::CheckedAddAndIncrement, Op::Cooperate, Op::Done];
fn bad(pos: Pos, reason: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", reason)
}
pub fn is_candidate(tokens: &[Token]) -> bool {
    matches!(tokens.first().map(|x| &x.tok), Some(Tok::Ident(s)) if s == "define")
    && matches!(tokens.get(1).map(|x| &x.tok), Some(Tok::Ident(s)) if s == "actor")
    && tokens.windows(2).any(|w| matches!(
        (&w[0].tok, &w[1].tok),
        (Tok::Ident(a), Tok::Ident(b)) if a == "rt" && b == "cooperate"))
}
struct Cursor<'a> { tokens: &'a [Token], idx: usize }
impl Cursor<'_> {
    fn pos(&self) -> Pos {
        self.tokens.get(self.idx).or_else(|| self.tokens.last())
            .map(|t| t.pos).unwrap_or(Pos { line:1, col:1 })
    }
    fn take(&mut self) -> Result<Tok, Diagnostic> {
        let t = self.tokens.get(self.idx)
            .ok_or_else(|| bad(self.pos(), "unexpected EOF in cooperative actor"))?;
        self.idx += 1;
        Ok(t.tok.clone())
    }
    fn expect(&mut self, token: Tok) -> Result<(), Diagnostic> {
        if self.tokens.get(self.idx).is_some_and(|t| t.tok == token) {
            self.idx += 1; Ok(())
        } else { Err(bad(self.pos(), format!("expected {token:?}"))) }
    }
    fn word(&mut self, s: &str) -> Result<(), Diagnostic> {
        match self.take()? {
            Tok::Ident(x) if x == s => Ok(()),
            _ => Err(bad(self.pos(), format!("expected keyword or name {s}"))),
        }
    }
    fn name(&mut self) -> Result<String, Diagnostic> {
        match self.take()? {
            Tok::Ident(n) if !n.is_empty()
                && n.as_bytes()[0].is_ascii_alphabetic()
                && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !["rt","cooperate","int","let","while","return","actor",
                     "pub","fnc","define"].contains(&n.as_str()) => Ok(n),
            _ => Err(bad(self.pos(), "invalid M8 identifier")),
        }
    }
    fn number(&mut self) -> Result<i64, Diagnostic> {
        let minus = self.tokens.get(self.idx).is_some_and(|t| t.tok == Tok::Minus);
        if minus { self.idx += 1; }
        let at = self.pos();
        match self.take()? {
            Tok::Int(Some(n)) => {
                if minus && n == (i64::MAX as u64) + 1 { return Ok(i64::MIN); }
                let v = i64::try_from(n).map_err(|_| bad(at, "i64 literal overflow"))?;
                Ok(if minus { -v } else { v })
            }
            _ => Err(bad(at, "signed i64 literal required")),
        }
    }
}
pub fn lower(tokens: &[Token]) -> Result<Plan, Diagnostic> {
    if tokens.is_empty() || tokens.len() > MAX_TOKENS {
        return Err(bad(tokens.first().map(|t| t.pos).unwrap_or(Pos{line:1,col:1}),
                       "M8 actor token count invalid"));
    }
    let mut c = Cursor { tokens, idx: 0 };
    c.word("define")?; c.word("actor")?; let actor = c.name()?;
    c.word("as")?; c.expect(Tok::LBrace)?;
    c.word("pub")?; c.word("fnc")?; c.word("on_message")?;
    c.expect(Tok::LParen)?; c.word("int")?; let amount = c.name()?;
    c.expect(Tok::RParen)?; c.expect(Tok::Colon)?; c.word("int")?;
    c.expect(Tok::LBrace)?;
    c.word("let")?; let total = c.name()?; c.expect(Tok::Colon)?;
    c.word("int")?; c.expect(Tok::Assign)?; let initial_total=c.number()?;
    c.expect(Tok::Semi)?;
    c.word("let")?; let index = c.name()?; c.expect(Tok::Colon)?;
    c.word("int")?; c.expect(Tok::Assign)?; let initial_index=c.number()?;
    c.expect(Tok::Semi)?;
    if amount == total || amount == index || total == index || initial_index < 0 {
        return Err(bad(c.pos(), "unsafe alias or negative loop index"));
    }
    c.word("while")?; c.expect(Tok::LParen)?; c.word(&index)?;
    c.expect(Tok::Lt)?; c.word(&amount)?; c.expect(Tok::RParen)?;
    c.expect(Tok::LBrace)?;
    c.word(&total)?; c.expect(Tok::Assign)?; c.word(&total)?;
    c.expect(Tok::Plus)?; c.word(&index)?; c.expect(Tok::Semi)?;
    c.word(&index)?; c.expect(Tok::Assign)?; c.word(&index)?;
    c.expect(Tok::Plus)?;
    if c.number()? != 1 { return Err(bad(c.pos(), "M8 requires checked +1 index")); }
    c.expect(Tok::Semi)?; c.word("rt")?; c.word("cooperate")?;
    c.expect(Tok::Semi)?; c.expect(Tok::RBrace)?;
    c.word("return")?; c.word(&total)?; c.expect(Tok::Semi)?;
    c.expect(Tok::RBrace)?; c.expect(Tok::RBrace)?; c.expect(Tok::Eof)?;
    if c.idx != tokens.len() { return Err(bad(c.pos(), "unexpected trailing source")); }
    Ok(Plan { actor, initial_total, initial_index })
}
fn uleb(out: &mut Vec<u8>, mut n:u64) {
    loop { let v=(n&127)as u8; n>>=7;
        if n==0 {out.push(v);return;} out.push(v|128); }
}
fn sleb(out:&mut Vec<u8>,mut n:i64){
    loop {let b=(n&127)as u8;n>>=7;
        let done=(n==0 && b&64==0)||(n== -1 && b&64!=0);
        out.push(if done{b}else{b|128});if done{return;}
    }
}
fn section(out:&mut Vec<u8>,id:u8,data:Vec<u8>){
    out.push(id);uleb(out,data.len()as u64);out.extend(data);
}
fn name(out:&mut Vec<u8>,s:&str){
    uleb(out,s.len()as u64);out.extend_from_slice(s.as_bytes());
}
struct Body(Vec<u8>);
impl Body{
    fn new()->Self{Self(vec![0])} // no locals
    fn op(&mut self,n:u8){self.0.push(n);}
    fn i32(&mut self,v:i32){self.op(0x41);sleb(&mut self.0,i64::from(v));}
    fn i64(&mut self,v:i64){self.op(0x42);sleb(&mut self.0,v);}
    fn get(&mut self,n:u32){self.op(0x20);uleb(&mut self.0,n as u64);}
    fn set(&mut self,n:u32){self.op(0x21);uleb(&mut self.0,n as u64);}
    fn gg(&mut self,n:u32){self.op(0x23);uleb(&mut self.0,n as u64);}
    fn gs(&mut self,n:u32){self.op(0x24);uleb(&mut self.0,n as u64);}
    fn if_(&mut self){self.op(0x04);self.op(0x40);}
    fn loop_(&mut self){self.op(0x03);self.op(0x40);}
    fn end(&mut self){self.op(0x0b);}
    fn trap_if(&mut self){self.if_();self.op(0x00);self.end();}
    fn handle(&mut self){
        self.get(0);self.gg(0);self.op(0x52);self.trap_if(); // invalid epoch
        self.gg(0);self.i64(0);self.op(0x51);self.trap_if(); // zero never live
    }
    fn done(mut self)->Vec<u8>{self.end();self.0}
}
pub fn emit(plan:&Plan)->Vec<u8>{
    // Each Wasm instance owns one frame: epoch i64, status i32, amount i64,
    // total i64, index i64, resume_pc i32. No imports or host callbacks.
    // status: EMPTY=0, SUSPENDED=1, DONE=2, RUNNING=3.
    let mut funcs=Vec::new();
    // m8_new(amount) -> generation handle
    let mut b=Body::new();
    b.gg(1);b.op(0x45);b.op(0x45);b.trap_if();
    b.gg(0);b.i64(i64::MAX);b.op(0x51);b.trap_if();
    b.gg(0);b.i64(1);b.op(0x7c);b.gs(0);
    b.get(0);b.gs(2);b.i64(plan.initial_total);b.gs(3);
    b.i64(plan.initial_index);b.gs(4);b.i32(1);b.gs(1);
    b.i32(0);b.gs(5);b.gg(0);funcs.push(b.done());
    // m8_resume(epoch,budget) -> YIELD=1 DONE=0.
    let mut b=Body::new();
    b.handle();
    b.get(1);b.i32(1);b.op(0x48);b.trap_if();
    b.get(1);b.i32(MAX_REDUCTIONS);b.op(0x4a);b.trap_if();
    b.gg(1);b.i32(1);b.op(0x46);
    b.gg(1);b.i32(2);b.op(0x46);b.op(0x72);b.op(0x45);b.trap_if();
    b.gg(1);b.i32(2);b.op(0x46);b.if_();
    b.i32(0);b.op(0x0f);b.end();
    b.i32(3);b.gs(1);b.loop_();
    b.gg(4);b.gg(2);b.op(0x59);b.if_(); // index >= amount
    b.i32(2);b.gs(1);b.i32(4);b.gs(5);
    b.i32(0);b.op(0x0f);b.end();
    b.get(1);b.op(0x45);b.if_();
    b.i32(1);b.gs(1);b.i32(1);b.gs(5);
    b.i32(1);b.op(0x0f);b.end();
    // Validate both arithmetic operations BEFORE mutating persistent slots.
    // Nonnegative index and i64_MAX-index cannot underflow.
    b.gg(3);b.i64(i64::MAX);b.gg(4);b.op(0x7d);b.op(0x55);b.trap_if();
    b.gg(4);b.i64(i64::MAX);b.op(0x51);b.trap_if();
    b.gg(3);b.gg(4);b.op(0x7c);b.gs(3);
    b.gg(4);b.i64(1);b.op(0x7c);b.gs(4);
    b.get(1);b.i32(1);b.op(0x6b);b.set(1);
    b.i32(1);b.gs(5);b.op(0x0c);uleb(&mut b.0,0);
    b.end();b.op(0x00);funcs.push(b.done());
    // m8_result(handle)->i64, only after DONE.
    let mut b=Body::new();b.handle();
    b.gg(1);b.i32(2);b.op(0x47);b.trap_if();b.gg(3);
    funcs.push(b.done());
    // m8_pc(handle)->i32 while SUSPENDED/DONE.
    let mut b=Body::new();b.handle();
    b.gg(1);b.i32(1);b.op(0x46);b.gg(1);b.i32(2);
    b.op(0x46);b.op(0x72);b.op(0x45);b.trap_if();
    b.gg(5);funcs.push(b.done());
    // m8_close(handle)->void, no double close or running-frame reclaim.
    let mut b=Body::new();b.handle();
    b.gg(1);b.i32(1);b.op(0x46);b.gg(1);b.i32(2);
    b.op(0x46);b.op(0x72);b.op(0x45);b.trap_if();
    b.i32(0);b.gs(1);b.i32(0);b.gs(5);
    b.i64(0);b.gs(2);b.i64(0);b.gs(3);b.i64(0);b.gs(4);
    funcs.push(b.done());
    // m8_index(handle)->i64, deterministic no-replay evidence.
    let mut b=Body::new();b.handle();
    b.gg(1);b.i32(1);b.op(0x46);b.gg(1);b.i32(2);
    b.op(0x46);b.op(0x72);b.op(0x45);b.trap_if();
    b.gg(4);funcs.push(b.done());

    let mut out=b"\0asm\x01\0\0\0".to_vec();
    let sigs:&[(&[u8],&[u8])]=&[
        (&[0x7e],&[0x7e]),(&[0x7e,0x7f],&[0x7f]),
        (&[0x7e],&[0x7f]),(&[0x7e],&[]),
    ];
    let mut types=Vec::new();uleb(&mut types,sigs.len()as u64);
    for &(args,rets) in sigs{
        types.push(0x60);uleb(&mut types,args.len()as u64);types.extend(args);
        uleb(&mut types,rets.len()as u64);types.extend(rets);
    }
    section(&mut out,1,types);
    let mut functions=Vec::new();uleb(&mut functions,6);
    for i in [0_u32,1,0,2,3,0]{uleb(&mut functions,i as u64);}
    section(&mut out,3,functions);
    let mut globals=Vec::new();uleb(&mut globals,6);
    for t in [0x7e_u8,0x7f,0x7e,0x7e,0x7e,0x7f]{
        globals.push(t);globals.push(1);
        if t==0x7e{globals.push(0x42);}else{globals.push(0x41);}
        sleb(&mut globals,0);globals.push(0x0b);
    }
    section(&mut out,6,globals);
    let mut exports=Vec::new();uleb(&mut exports,6);
    for (i,s) in ["m8_new","m8_resume","m8_result","m8_pc","m8_close","m8_index"].iter().enumerate(){
        name(&mut exports,s);exports.push(0);uleb(&mut exports,i as u64);
    }
    section(&mut out,7,exports);
    let mut bodies=Vec::new();uleb(&mut bodies,6);
    for f in funcs{uleb(&mut bodies,f.len()as u64);bodies.extend(f);}
    section(&mut out,10,bodies);
    let mut meta=Vec::new();name(&mut meta,"atto.m8.cooperate");
    name(&mut meta,&plan.actor);uleb(&mut meta,IR.len()as u64);
    section(&mut out,0,meta);out
}
