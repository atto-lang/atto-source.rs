//! Closed nominal Point placement profile, independently checked before emission.
//! One private fixed Wasm region, no host calls, raw address, arbitrary layout,
//! user destructor, actor, reentrant call or dynamic allocator authority.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Step {
    Construct(i64,i64), Bind, CopyField(u8), Detach, Destroy, Free,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
    pub result: Vec<usize>,
}
fn err(pos: Pos) -> Diagnostic {
    Diagnostic::new(pos,"E_POINTER_PLACEMENT","invalid closed typed Point placement profile")
}
impl Plan {
    pub(crate) fn verify(&self) -> Result<usize,Diagnostic> {
        let bad=||err(Pos::default());
        if self.steps.is_empty() || self.steps.len()>32 || self.result.is_empty() || self.result.len()>2 {
            return Err(bad());
        }
        #[derive(PartialEq,Eq)]
        enum State { Empty, Live, Freed }
        let mut state=State::Empty;
        let mut generation=0u32;
        let mut bound=None;
        let mut fields=[0i64;2];
        let mut copies=Vec::new();
        for step in &self.steps {
            match *step {
                Step::Construct(x,y) if state==State::Empty && bound.is_none() && x>=0 && y>=0 => {
                    generation=generation.checked_add(1).ok_or_else(bad)?;
                    fields=[x,y]; state=State::Live;
                }
                Step::Bind if state==State::Live && bound.is_none() => bound=Some(generation),
                Step::CopyField(f) if state==State::Live && bound==Some(generation) && f<2 => copies.push(fields[f as usize]),
                Step::Detach if state==State::Live && bound==Some(generation) => bound=None,
                Step::Destroy if state==State::Live && bound.is_none() => state=State::Empty,
                Step::Free if state==State::Empty && bound.is_none() => state=State::Freed,
                _ => return Err(bad()),
            }
        }
        if state!=State::Freed || bound.is_some() {return Err(bad());}
        let mut sum=0i64;
        for i in &self.result {
            sum=sum.checked_add(*copies.get(*i).ok_or_else(bad)?).ok_or_else(bad)?;
        }
        Ok(copies.len())
    }
}
struct Parser<'a> { tokens:&'a [Token], at:usize }
impl Parser<'_> {
    fn token(&self)->&Token { &self.tokens[self.at.min(self.tokens.len()-1)] }
    fn word_is(&self,w:&str)->bool { matches!(&self.token().tok,Tok::Ident(n) if n==w) }
    fn word(&mut self,w:&str)->Result<(),Diagnostic> {
        if !self.word_is(w) {return Err(err(self.token().pos));} self.at+=1; Ok(())
    }
    fn take(&mut self,t:Tok)->bool { if self.token().tok==t {self.at+=1;true}else{false} }
    fn need(&mut self,t:Tok)->Result<(),Diagnostic> {
        if self.take(t){Ok(())}else{Err(err(self.token().pos))}
    }
    fn op(&mut self,w:&str)->Result<(),Diagnostic> {
        let p=self.token().pos;self.word(w)?;
        let b=self.token().pos;self.need(Tok::Bang)?;
        if p.line!=b.line || b.col!=p.col+w.len() as u32 {return Err(err(b));} Ok(())
    }
    fn number(&mut self)->Result<i64,Diagnostic> {
        let t=self.token();if let Tok::Int(Some(n))=t.tok {
            if let Ok(v)=i64::try_from(n){self.at+=1;return Ok(v);}
        } Err(err(t.pos))
    }
    fn ident(&mut self)->Result<String,Diagnostic> {
        let t=self.token();
        if let Tok::Ident(n)=&t.tok {
            if n.len()<=64 && n.starts_with(|c:char|c.is_ascii_lowercase())
                && n.chars().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c=='_')
                && !crate::parser::is_reserved(n)
                && !matches!(n.as_str(),"s"|"p"|"x"|"y"|"main"|"int"|"alloc"|"destroy"|"free"|"ref"|"deref"|"unref"|"at"|"to"|"empty"|"null"|"true"|"false"|"self"|"constructor") {
                let n=n.clone();self.at+=1;return Ok(n);
            }
        } Err(err(t.pos))
    }
    fn parse(&mut self)->Result<Plan,Diagnostic> {
        use Tok::*;
        self.word("define")?;self.word("struct")?;self.word("Point")?;self.need(LBrace)?;
        for f in ["x","y"] {self.word("int")?;self.word(f)?;self.need(Semi)?;}
        self.need(RBrace)?;self.word("with")?;self.need(LBrace)?;self.word("constructor")?;
        self.need(LParen)?;self.word("int")?;self.word("x")?;self.need(Comma)?;
        self.word("int")?;self.word("y")?;self.need(RParen)?;self.need(LBrace)?;
        for f in ["x","y"] {self.word("self")?;self.need(Dot)?;self.word(f)?;self.need(Assign)?;self.word(f)?;self.need(Semi)?;}
        self.need(RBrace)?;self.need(RBrace)?;
        self.word("pub")?;self.word("fnc")?;self.word("main")?;self.need(LParen)?;self.need(RParen)?;
        self.need(Colon)?;self.word("int")?;self.need(LBrace)?;
        self.word("let")?;self.word("Storage")?;self.need(Lt)?;self.word("Point")?;self.need(Gt)?;
        self.word("s")?;self.need(Assign)?;self.op("alloc")?;self.word("Point")?;self.need(Semi)?;
        self.word("let")?;self.word("mut")?;self.word("RawPointer")?;self.need(Lt)?;self.word("Point")?;
        self.need(Gt)?;self.word("p")?;self.need(Assign)?;self.word("RawPointer")?;self.need(Dot)?;
        self.word("empty")?;self.need(Lt)?;self.word("Point")?;self.need(Gt)?;self.need(LParen)?;self.need(RParen)?;self.need(Semi)?;
        let mut steps=Vec::new();let mut names=BTreeMap::new();
        while !self.word_is("return") {
            if steps.len()>=32{return Err(err(self.token().pos));}
            let step=if self.word_is("new") {
                self.word("new")?;self.word("Point")?;self.need(LParen)?;let x=self.number()?;
                self.need(Comma)?;let y=self.number()?;self.need(RParen)?;self.word("at")?;self.word("s")?;Step::Construct(x,y)
            } else if self.word_is("ref") {
                self.op("ref")?;self.word("p")?;self.word("to")?;self.word("s")?;Step::Bind
            } else if self.word_is("val") {
                self.word("val")?;let n=self.ident()?;
                let i=names.len();if names.insert(n,i).is_some(){return Err(err(self.token().pos));}
                self.need(Assign)?;self.need(LParen)?;self.op("deref")?;self.word("p")?;self.need(RParen)?;self.need(Dot)?;
                if self.word_is("x"){self.word("x")?;Step::CopyField(0)}else{self.word("y")?;Step::CopyField(1)}
            } else if self.word_is("unref") {self.op("unref")?;self.word("p")?;Step::Detach}
            else if self.word_is("destroy"){self.op("destroy")?;self.word("s")?;Step::Destroy}
            else if self.word_is("free"){self.op("free")?;self.word("s")?;Step::Free}
            else{return Err(err(self.token().pos));};
            self.need(Semi)?;steps.push(step);
        }
        self.word("return")?;let mut result=Vec::new();
        loop {
            if result.len()==2{return Err(err(self.token().pos));}
            let n=self.ident()?;result.push(*names.get(&n).ok_or_else(||err(self.token().pos))?);
            if !self.take(Plus){break;}
        }
        self.need(Semi)?;self.need(RBrace)?;self.need(Eof)?;
        if self.at!=self.tokens.len(){return Err(err(self.token().pos));}
        let p=Plan{steps,result};p.verify()?;Ok(p)
    }
}
pub(crate) fn try_plan(tokens:&[Token])->Result<Option<Plan>,Diagnostic> {
    let requests=tokens.windows(2).any(|w|matches!((&w[0].tok,&w[1].tok),(Tok::Ident(s),Tok::Lt) if s=="Storage"));
    if !requests{return Ok(None);}
    Parser{tokens,at:0}.parse().map(Some)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutated_typed_plans_fail_before_emission(){
        let good=Plan{steps:vec![Step::Construct(40,2),Step::Bind,Step::CopyField(1),Step::Detach,Step::Destroy,Step::Free],result:vec![0]};
        assert!(crate::wasm::emit_point_placement(&good).is_ok());
        let mut bad=good.clone();bad.steps[2]=Step::CopyField(2);
        assert!(crate::wasm::emit_point_placement(&bad).is_err());
        let mut bad=good.clone();bad.steps[3]=Step::Destroy;
        assert!(crate::wasm::emit_point_placement(&bad).is_err());
        let mut bad=good.clone();bad.result[0]=9;
        assert!(crate::wasm::emit_point_placement(&bad).is_err());
        let mut bad=good;bad.steps.pop();
        assert!(crate::wasm::emit_point_placement(&bad).is_err());
    }
}
