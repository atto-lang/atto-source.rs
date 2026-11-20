//! A tightly bounded source-to-Wasm List<int> match profile.
//! Actual List elements live in guest linear memory; matching uses checked
//! typed CollectionLoad, If and Return IR, NOT a host interpreter or a
//! compile-time synthesized numeric result. General List/Array, runtime
//! fragment objects and arbitrary guards remain E_UNSUPPORTED.

use crate::ast::{BinOp, Type};
use crate::check::{TExpr, TFunc, TProgram, TStmt, TE};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use std::collections::BTreeSet;

const MAX_VALUES: usize = 32;
const MAX_FIXED: usize = 16;
type R<T> = Result<T, Diagnostic>;

fn err(pos: Pos, code: &'static str, message: &'static str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn unsupported(pos: Pos) -> Diagnostic {
    err(pos, "E_UNSUPPORTED", "List<int> guest profile cannot lower this construct")
}
struct Cursor<'a> { tokens: &'a [Token], at: usize }
impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token]) -> Self { Self { tokens, at: 0 } }
    fn pos(&self) -> Pos {
        self.tokens.get(self.at).map_or(Pos::default(), |t| t.pos)
    }
    fn peek(&self) -> &Tok { &self.tokens[self.at.min(self.tokens.len()-1)].tok }
    fn eat(&mut self, tok: Tok) -> bool {
        if *self.peek() != tok { return false; }
        if tok != Tok::Eof { self.at += 1; }
        true
    }
    fn word(&mut self, expected: &str) -> bool {
        match self.peek() {
            Tok::Ident(name) if name == expected => { self.at += 1; true }
            _ => false
        }
    }
    fn expect(&mut self, tok: Tok) -> R<()> {
        if self.eat(tok) { Ok(()) } else { Err(unsupported(self.pos())) }
    }
    fn expect_word(&mut self, text: &str) -> R<()> {
        if self.word(text) { Ok(()) } else { Err(unsupported(self.pos())) }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Ident(name) if name != "_" => { self.at += 1; Ok(name) }
            _ => Err(unsupported(self.pos()))
        }
    }
    fn i64_literal(&mut self) -> R<i64> {
        let pos = self.pos();
        let negative = self.eat(Tok::Minus);
        match self.peek().clone() {
            Tok::Int(Some(value))
                if (!negative && value <= i64::MAX as u64) ||
                    (negative && value <= i64::MAX as u64 + 1) => {
                self.at += 1;
                if negative && value == i64::MAX as u64 + 1 { Ok(i64::MIN) }
                else if negative { Ok(-(value as i64)) }
                else { Ok(value as i64) }
            }
            _ => Err(err(pos,"E_INTEGER_RANGE","invalid signed i64 literal"))
        }
    }
}
#[derive(Clone,Debug)]
enum Fixed { Any, Literal(i64), Capture(String) }
#[derive(Clone,Debug)]
enum Term { Capture(String), Literal(i64) }
#[derive(Clone)]
struct Arm {
    leading: bool, trailing: bool, items: Vec<Fixed>,
    terms: Vec<Term>, guard: Option<(String,BinOp,i64)>, fallback: i64,
}
fn match_pattern(p: &mut Cursor<'_>) -> R<(bool,bool,Vec<Fixed>)> {
    p.expect_word("List")?;
    p.expect(Tok::Dot)?;
    p.expect_word("of")?;
    p.expect(Tok::LParen)?;
    let mut seq = Vec::new();
    if !p.eat(Tok::RParen) {
        loop {
            if seq.len() >= MAX_FIXED + 2 {
                return Err(err(p.pos(),"E_PATTERN_BUDGET","too many pattern items"));
            }
            let token = if p.eat(Tok::Ellipsis) {
                if p.eat(Tok::LParen) { return Err(unsupported(p.pos())); }
                None
            } else if p.word("_") {
                Some(Fixed::Any)
            } else if p.word("is") {
                p.expect_word("int")?;
                if p.word("as") {
                    p.expect_word("val")?;
                    Some(Fixed::Capture(p.ident()?))
                } else { Some(Fixed::Any) }
            } else if matches!(p.peek(),Tok::Int(_) | Tok::Minus) {
                Some(Fixed::Literal(p.i64_literal()?))
            } else { return Err(unsupported(p.pos())); };
            seq.push(token);
            if p.eat(Tok::RParen) { break; }
            p.expect(Tok::Comma)?;
            if p.peek() == &Tok::RParen {
                return Err(err(p.pos(),"E_PATTERN_SOURCE","trailing pattern comma"));
            }
        }
    }
    let leading = matches!(seq.first(),Some(None));
    let trailing = seq.len()>1 && matches!(seq.last(),Some(None));
    let rests = seq.iter().filter(|x| x.is_none()).count();
    if rests > usize::from(leading) + usize::from(trailing) ||
        (rests == 2 && seq.len() == 2) {
        return Err(unsupported(p.pos()));
    }
    let items: Vec<Fixed> = seq.into_iter().flatten().collect();
    let mut names = BTreeSet::new();
    for item in &items {
        if let Fixed::Capture(name) = item {
            if !names.insert(name.clone()) {
                return Err(err(p.pos(),"E_PATTERN_BINDING","duplicate pattern capture"));
            }
        }
    }
    Ok((leading,trailing,items))
}
fn terms(p: &mut Cursor<'_>, allowed: &BTreeSet<String>) -> R<Vec<Term>> {
    let mut result = Vec::new();
    loop {
        if result.len() == MAX_FIXED {
            return Err(err(p.pos(),"E_PATTERN_BUDGET","too many result operands"));
        }
        match p.peek().clone() {
            Tok::Ident(name) if allowed.contains(&name) => {
                p.at += 1; result.push(Term::Capture(name));
            }
            Tok::Ident(_) => {
                return Err(err(p.pos(),"E_PATTERN_BINDING","unbound arm variable"));
            }
            Tok::Int(_) | Tok::Minus => result.push(Term::Literal(p.i64_literal()?)),
            _ => return Err(unsupported(p.pos())),
        }
        if !p.eat(Tok::Plus) { break; }
    }
    Ok(result)
}
fn parse(p: &mut Cursor<'_>) -> R<(Vec<i64>,Arm)> {
    p.word("pub");
    p.expect_word("fnc")?;
    p.expect_word("main")?;
    p.expect(Tok::LParen)?;
    p.expect(Tok::RParen)?;
    if !p.eat(Tok::Colon) { p.expect(Tok::Arrow)?; }
    p.expect_word("int")?;
    p.expect(Tok::LBrace)?;
    p.expect_word("val")?;
    let name = p.ident()?;
    p.expect(Tok::Colon)?;
    p.expect_word("List")?;
    p.expect(Tok::Lt)?;
    p.expect_word("int")?;
    p.expect(Tok::Gt)?;
    p.expect(Tok::Assign)?;
    p.expect_word("List")?;
    p.expect(Tok::Dot)?;
    p.expect_word("of")?;
    p.expect(Tok::LParen)?;
    let mut values=Vec::new();
    if !p.eat(Tok::RParen) {
        loop {
            if values.len() >= MAX_VALUES {
                return Err(err(p.pos(),"E_PATTERN_BUDGET","too many List elements"));
            }
            values.push(p.i64_literal()?);
            if p.eat(Tok::RParen) { break; }
            p.expect(Tok::Comma)?;
            if p.peek() == &Tok::RParen {
                return Err(err(p.pos(),"E_PATTERN_SOURCE","trailing constructor comma"));
            }
        }
    }
    p.expect(Tok::Semi)?;
    p.expect_word("return")?;
    p.expect_word("match")?;
    p.expect(Tok::Lt)?;
    p.expect_word("int")?;
    p.expect(Tok::Gt)?;
    p.expect_word(&name)?;
    p.expect_word("over")?;
    p.expect_word("on")?;
    let (leading,trailing,items) = match_pattern(p)?;
    let names: BTreeSet<String> = items.iter().filter_map(|item|
        if let Fixed::Capture(name) = item {Some(name.clone())} else {None}
    ).collect();
    let guard=if p.word("when") {
        let variable=p.ident()?;
        if !names.contains(&variable) {
            return Err(err(p.pos(),"E_PATTERN_BINDING","guard capture not bound"));
        }
        let op=match p.peek() {
            Tok::Gt => BinOp::Gt, Tok::Ge => BinOp::Ge,
            Tok::Lt => BinOp::Lt, Tok::Le => BinOp::Le,
            Tok::EqEq => BinOp::Eq, Tok::NotEq => BinOp::Ne,
            _ => return Err(unsupported(p.pos()))
        };
        p.at+=1;
        Some((variable,op,p.i64_literal()?))
    } else {None};
    p.expect(Tok::Arrow)?;
    let terms=terms(p,&names)?;
    p.expect(Tok::Comma)?;
    p.expect_word("on")?;
    p.expect_word("_")?;
    p.expect(Tok::Arrow)?;
    let fallback=p.i64_literal()?;
    p.expect_word("end")?;
    p.expect(Tok::Semi)?;
    p.expect(Tok::RBrace)?;
    p.expect(Tok::Eof)?;
    Ok((values,Arm {leading,trailing,items,terms,guard,fallback}))
}
fn num(v: i64) -> TE { TE {e:TExpr::I64(v),ty:Type::Int} }
fn truth(v:bool) -> TE { TE {e:TExpr::Bool(v),ty:Type::Bool} }
fn owner() -> TE { TE {e:TExpr::Local(0),ty:Type::Array(0)} }
fn cell(i: usize) -> TE {
    TE {e:TExpr::CollectionLoad {
        array:Box::new(owner()),offset:((i+1)*8) as u32
    },ty:Type::Int}
}
fn binary(op:BinOp,left:TE,right:TE,ty:Type)->TE{
    TE{e:TExpr::Bin(op,Box::new(left),Box::new(right)),ty}
}
fn captured(term:&Term, items:&[Fixed],start:usize)->TE{
    match term{
        Term::Literal(v)=>num(*v),
        Term::Capture(name)=>{
            let index=items.iter().position(|a|
                matches!(a,Fixed::Capture(other) if other==name))
                .expect("parser validates every capture");
            cell(start+index)
        }
    }
}
fn build(values:&[i64],arm:&Arm)->R<TProgram>{
    let n=values.len();
    let mut body=vec![TStmt::Set(0,TE{
        e:TExpr::CollectionAlloc(((n+1)*8) as u32),ty:Type::Array(0)
    })];
    body.push(TStmt::CollectionStore {array:owner(),offset:0,value:num(n as i64)});
    for (i,v) in values.iter().enumerate(){
        body.push(TStmt::CollectionStore{
            array:owner(),offset:((i+1)*8) as u32,value:num(*v)
        });
    }
    let m=arm.items.len();
    let mut starts=Vec::new();
    if m<=n {
        for start in 0..=n-m {
            if !arm.leading && start!=0 {continue;}
            if !arm.trailing && start+m!=n {continue;}
            starts.push(start);
        }
    }
    if starts.len()>MAX_VALUES{
        return Err(err(Pos::default(),"E_PATTERN_BUDGET","matcher branch budget exceeded"));
    }
    let default=||vec![TStmt::Return(Some(num(arm.fallback)))];
    let mut next=default();
    // Reverse construction means first matching start always wins.
    // Once a witness is selected a failed guard exits to default,
    // never searches a later decomposition.
    for start in starts.into_iter().rev(){
        let mut matched=truth(true);
        for (offset,item) in arm.items.iter().enumerate(){
            if let Fixed::Literal(value)=item {
                let check=binary(BinOp::Eq,cell(start+offset),num(*value),Type::Bool);
                matched=binary(BinOp::And,matched,check,Type::Bool);
            }
        }
        let mut sum:Option<TE>=None;
        for term in &arm.terms{
            let value=captured(term,&arm.items,start);
            sum=Some(match sum{
                Some(previous)=>binary(BinOp::Add,previous,value,Type::Int),
                None=>value
            });
        }
        let success=vec![TStmt::Return(Some(sum.unwrap_or_else(||num(0))))];
        let selected=if let Some((name,op,value))=&arm.guard{
            let condition=binary(*op,captured(&Term::Capture(name.clone()),
                &arm.items,start),num(*value),Type::Bool);
            vec![TStmt::If(vec![(condition,success)],Some(default()))]
        } else {success};
        next=vec![TStmt::If(vec![(matched,selected)],Some(next))];
    }
    body.extend(next);
    Ok(TProgram {
        funcs:vec![TFunc{name:"main".into(),params:vec![],ret:Type::Int,
            locals:vec![Type::Array(0)],body}],
        generators:vec![],main:0,imports:vec![],data:vec![],
        heap:true,globals:vec![],actors:vec![],
        strings:false,guest_eh:false,raise_tag:false,catch_depth:false
    })
}
/// Admit only an explicitly typed List<int> owned local, with the List.of
/// constructor. The pre-existing scalar compiler and array layout guest
/// profiles stay independent and unchanged.
pub fn try_lower(tokens:&[Token])->R<Option<TProgram>>{
    let typed=tokens.windows(4).any(|v|matches!(
        (&v[0].tok,&v[1].tok,&v[2].tok,&v[3].tok),
        (Tok::Ident(a),Tok::Lt,Tok::Ident(b),Tok::Gt)
        if a=="List" && b=="int"));
    let ctor=tokens.windows(3).any(|v|matches!(
        (&v[0].tok,&v[1].tok,&v[2].tok),
        (Tok::Ident(a),Tok::Dot,Tok::Ident(b))
        if a=="List" && b=="of"));
    if !typed||!ctor{return Ok(None);}
    let (values,arm)=parse(&mut Cursor::new(tokens))?;
    build(&values,&arm).map(Some)
}
