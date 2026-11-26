//! Narrow immutable scalar aggregate specialization, not a general aggregate ABI.
//! Only a single braced int main with leading aggregate literal bindings and
//! scalar projections is admitted. Each field becomes a real checked scalar
//! local; the ordinary typed parser, checker and Wasm emitter remain authoritative.
//! No whole aggregate, mutation, escape, runtime type object or computed field
//! expression is accepted. The name binding lives only in this callable.
use std::collections::BTreeMap;
use crate::diag::Diagnostic;
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;
const PREFIX: &str = "__atto_named_aggregate_";
const MAX_FIELDS: usize = 64;
const MAX_BINDINGS: usize = 64;

fn word(t: &Token) -> Option<&str> {
    match &t.tok { Tok::Ident(s) => Some(s), _ => None }
}
fn wrapper(t: &Token) -> Option<&str> {
    word(t).filter(|s| matches!(*s, "Record" | "Tuple" | "struct"))
}
fn err(t: &Token, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(t.pos, code, message)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scalar { Int, Bool, String }
impl Scalar {
    fn spelling(self) -> &'static str {
        match self { Self::Int=>"int", Self::Bool=>"bool", Self::String=>"string" }
    }
}
#[derive(Clone)]
struct Field { ty: Scalar, local: String }
struct Shape { kind: String, fields: BTreeMap<String, Scalar> }
struct Local { kind: String, fields: BTreeMap<String, Field> }
struct Parser<'a> {
    t: &'a [Token], i: usize, out: Vec<Token>,
    aliases: BTreeMap<String, Shape>, locals: BTreeMap<String, Local>,
    next: usize,
}
impl<'a> Parser<'a> {
    fn peek(&self) -> &'a Token { self.t.get(self.i).unwrap_or_else(|| &self.t[self.t.len()-1]) }
    fn take(&mut self) -> &'a Token { let t=self.peek(); if self.i<self.t.len(){self.i+=1;} t }
    fn eat(&mut self, tok: Tok) -> bool {
        if self.peek().tok==tok { self.take(); true } else { false }
    }
    fn expect(&mut self,tok: Tok) -> R<()> {
        if self.eat(tok) { Ok(()) } else { Err(err(self.peek(),"E_SYNTAX","malformed named aggregate literal or declaration")) }
    }
    fn expect_word(&mut self,s:&str) -> R<()> {
        if word(self.peek())==Some(s) { self.take(); Ok(()) }
        else { Err(err(self.peek(),"E_NAMED_AGGREGATE_PROFILE","expected bounded aggregate main or return")) }
    }
    fn id(&mut self) -> R<String> {
        let t=self.take();
        match word(t) {
            Some(s) if s.len()<=128 && !matches!(s,"infer"|"const"|"val"|"let"|"return"|"fnc"|"main"|"true"|"false"|"self"|"int"|"bool"|"string"|"float"|"void"|"Record"|"Tuple"|"struct"|"Any"|"Array"|"Option"|"Result"|"Map"|"List"|"Vector"|"Pair"|"Entry") => Ok(s.to_owned()),
            _=>Err(err(t,"E_TYPE_NAME","expected nonreserved identifier")),
        }
    }
    fn push(&mut self,tok:Tok,at:&Token) { self.out.push(Token{tok,pos:at.pos}); }
    fn literal(&mut self)->R<(Scalar,Vec<Token>)> {
        let at=self.take();
        match &at.tok {
            Tok::Int(Some(v)) if *v<=i64::MAX as u64 => Ok((Scalar::Int,vec![at.clone()])),
            Tok::Minus => {
                let v=self.take();
                if !matches!(&v.tok,Tok::Int(Some(n)) if *n<=(1u64<<63)) {
                    return Err(err(v,"E_TYPE_INFER_AMBIGUOUS","expected signed i64 scalar literal"));
                }
                Ok((Scalar::Int,vec![at.clone(),v.clone()]))
            }
            Tok::Str(_)=>Ok((Scalar::String,vec![at.clone()])),
            Tok::Ident(s) if matches!(s.as_str(),"true"|"false")=>Ok((Scalar::Bool,vec![at.clone()])),
            _=>Err(err(at,"E_TYPE_INFER_AMBIGUOUS","named aggregate profile requires bool/string/int literals")),
        }
    }
    fn bind(&mut self)->R<()> {
        let at=self.peek();
        self.expect_word("const")?;
        let kind=wrapper(self.take()).ok_or_else(||err(at,"E_NAMED_AGGREGATE_PROFILE","expected aggregate annotation"))?.to_owned();
        let (introduce,alias)=if self.eat(Tok::Lt) {
            let introduce=word(self.peek())==Some("infer");
            if introduce { self.take(); }
            let alias=self.id()?;
            self.expect(Tok::Gt)?;
            (introduce,Some(alias))
        } else { (false,None) };
        if let Some(name)=&alias {
            if introduce && self.aliases.contains_key(name) {
                return Err(err(at,"E_DUPLICATE_TYPE","duplicate named inferred type"));
            }
            if !introduce && !self.aliases.contains_key(name) {
                return Err(err(at,"E_UNKNOWN_TYPE","unknown named inferred type"));
            }
        }
        let name=self.id()?;
        if self.locals.contains_key(&name) {
            return Err(err(at,"E_DUPLICATE","duplicate aggregate local"));
        }
        if self.locals.len()>=MAX_BINDINGS {
            return Err(err(at,"E_SOURCE_LIMIT","named aggregate binding limit"));
        }
        self.expect(Tok::Assign)?;
        let close=if kind=="struct" { self.expect(Tok::LBrace)?; Tok::RBrace }
                  else { self.expect(Tok::LParen)?; Tok::RParen };
        let mut shape=BTreeMap::new();
        let mut fields=BTreeMap::new();
        // Keep value evaluation order independent of canonical shape order.
        let mut pending=Vec::new();
        loop {
            if fields.len()>=MAX_FIELDS { return Err(err(at,"E_SOURCE_LIMIT","aggregate field limit")); }
            let field=if kind=="Tuple" { fields.len().to_string() } else { self.id()? };
            if kind!="Tuple" {
                self.expect(if kind=="struct" {Tok::Colon} else {Tok::Assign})?;
            }
            if fields.contains_key(&field) { return Err(err(at,"E_DUPLICATE_FIELD","duplicate aggregate field")); }
            let (ty,literal)=self.literal()?;
            let local=format!("{PREFIX}{}",self.next); self.next+=1;
            shape.insert(field.clone(),ty);
            fields.insert(field,Field{ty,local:local.clone()});
            pending.push((ty,local,literal));
            if !self.eat(Tok::Comma) { break; }
        }
        self.expect(close)?;
        self.expect(Tok::Semi)?;
        if let Some(alias)=alias {
            if introduce {
                self.aliases.insert(alias,Shape{kind:kind.clone(),fields:shape});
            } else {
                let expected=&self.aliases[&alias];
                if expected.kind!=kind { return Err(err(at,"E_AGGREGATE_KIND","named aggregate kind mismatch")); }
                if expected.fields!=shape { return Err(err(at,"E_TYPE_MISMATCH","named aggregate shape mismatch")); }
            }
        }
        for (ty,local,literal) in pending {
            self.push(Tok::Ident("const".into()),at);
            self.push(Tok::Ident(ty.spelling().into()),at);
            self.push(Tok::Ident(local),at);
            self.push(Tok::Assign,at);
            self.out.extend(literal);
            self.push(Tok::Semi,at);
        }
        self.locals.insert(name,Local{kind,fields});
        Ok(())
    }
    fn program(mut self)->R<Vec<Token>> {
        if word(self.peek())==Some("pub") { self.take(); }
        self.expect_word("fnc")?;
        self.expect_word("main")?;
        self.expect(Tok::LParen)?; self.expect(Tok::RParen)?;
        if !self.eat(Tok::Colon) { self.expect(Tok::Arrow)?; }
        self.expect_word("int")?; self.expect(Tok::LBrace)?;
        self.out.extend_from_slice(&self.t[..self.i]);
        while word(self.peek())==Some("const") { self.bind()?; }
        let ret=self.peek();
        self.expect_word("return")?;
        self.out.push(ret.clone());
        // Only scalar arithmetic and explicit statically indexed/ named
        // projections can observe an aggregate. No identity or storage escape.
        while self.peek().tok!=Tok::Semi {
            let at=self.take();
            match &at.tok {
                Tok::Ident(name) => {
                    let aggregate=self.locals.get(name)
                        .ok_or_else(||err(at,"E_NAMED_AGGREGATE_PROFILE","return expression requires a known aggregate projection"))?;
                    let kind=aggregate.kind.clone();
                    let field=if kind=="Tuple" {
                        self.expect(Tok::LBracket)?;
                        let index=self.take();
                        let n=match &index.tok {
                            Tok::Int(Some(n))=>*n,
                            _=>return Err(err(index,"E_TYPE","tuple projection requires a nonnegative static index")),
                        };
                        self.expect(Tok::RBracket)?;
                        n.to_string()
                    } else { self.expect(Tok::Dot)?; self.id()? };
                    let member=self.locals[name].fields.get(&field)
                        .ok_or_else(||err(at,"E_NO_FIELD","unknown aggregate member or tuple index"))?;
                    if member.ty!=Scalar::Int {
                        return Err(err(at,"E_TYPE","int main requires integer aggregate projections"));
                    }
                    let local=member.local.clone();
                    self.push(Tok::Ident(local),at);
                }
                Tok::Int(Some(_))|Tok::Plus|Tok::Minus|Tok::Star|Tok::Slash|Tok::LParen|Tok::RParen=>self.out.push(at.clone()),
                _=>return Err(err(at,"E_NAMED_AGGREGATE_PROFILE","unsupported aggregate use or return expression")),
            }
        }
        self.expect(Tok::Semi)?; self.expect(Tok::RBrace)?;
        if self.peek().tok!=Tok::Eof { return Err(err(self.peek(),"E_NAMED_AGGREGATE_PROFILE","named aggregate scalar profile supports one main only")); }
        self.out.extend_from_slice(&self.t[self.i-2..]);
        if self.out.len()>crate::lexer::MAX_TOKENS {
            return Err(err(self.peek(),"E_SOURCE_LIMIT","expanded scalar aggregate token budget"));
        }
        Ok(self.out)
    }
}

pub(crate) fn lower(tokens: Vec<Token>)->R<Vec<Token>> {
    // Only lexed source tokens count; strings/comments cannot activate the pass.
    let named=tokens.windows(3).any(|w|wrapper(&w[0]).is_some() && w[1].tok==Tok::Lt && word(&w[2])==Some("infer"));
    let bare=tokens.windows(4).any(|w|word(&w[0])==Some("const") && wrapper(&w[1]).is_some() && word(&w[2]).is_some() && w[3].tok==Tok::Assign);
    if !named && !bare { return Ok(tokens); }
    if let Some(t)=tokens.iter().find(|t|word(t).is_some_and(|s|s.starts_with(PREFIX))) {
        return Err(err(t,"E_RESERVED","compiler-owned named aggregate local prefix"));
    }
    let result=Parser{t:&tokens,i:0,out:Vec::new(),aliases:BTreeMap::new(),locals:BTreeMap::new(),next:0}.program();
    // Existing non-binder source retains its previous compiler path if it
    // is outside this narrow anonymous aggregate specialization.
    if named { result } else { result.or(Ok(tokens)) }
}
