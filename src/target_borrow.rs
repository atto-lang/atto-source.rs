//! Path-sensitive ownership preflight for out-of-line actor receiver bodies.
//!
//! Only a restricted set of lexical ownership operations is modeled here.
//! Unrecognized control flow and expressions remain unproved. In particular,
//! these facts NEVER authorize executable AOT lowering or actor lease cleanup.
use std::collections::{BTreeMap, BTreeSet};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::target_syntax::ReceiverBodyTokens;
use crate::target_typed::{TypedActorModule, TypedReceiver};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiverBorrowFacts {
    pub receiver: usize,
    pub moves: usize,
    pub loans: usize,
    pub branch_joins: usize,
    pub lexical_loan_releases: usize,
    /// Only the supported subset has complete ownership-state coverage.
    /// Even true does not establish correct runtime unwinding/lease cleanup.
    pub ownership_preflight_complete: bool,
    pub normal_fallthrough: bool,
}

#[derive(Clone, Debug)]
struct Local {
    scope: usize,
    mutable: bool,
}

#[derive(Clone, Debug)]
struct Loan {
    source: String,
    exclusive: bool,
    scope: usize,
}

#[derive(Clone, Default)]
struct State {
    locals: BTreeMap<String, Local>,
    loans: BTreeMap<String, Loan>,
    moved: BTreeSet<String>,
}

#[derive(Default)]
struct Flow {
    falls_through: bool,
    complete: bool,
}

struct Reader<'a> {
    toks: &'a [Token],
    i: usize,
    receiver: &'a TypedReceiver,
    moves: usize,
    loans: usize,
    joins: usize,
    releases: usize,
}

fn named(t: &Token, name: &str) -> bool {
    matches!(&t.tok, Tok::Ident(s) if s == name)
}
fn word(toks: &[Token], i: usize, s: &str) -> bool {
    toks.get(i).is_some_and(|t| named(t,s))
}
fn is_call(toks: &[Token], i: usize) -> bool {
    matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Ident(_)))
        && matches!(toks.get(i+1).map(|t| &t.tok), Some(Tok::LParen))
}

impl Reader<'_> {
    fn pos(&self) -> Pos {
        self.toks.get(self.i).map_or(Pos::default(), |t| t.pos)
    }
    fn err(&self, code: &'static str, msg: &'static str) -> Diagnostic {
        Diagnostic::new(self.pos(), code, msg)
    }
    fn known(&self, state: &State, name: &str) -> bool {
        state.locals.contains_key(name)
            || self.receiver.params.iter().any(|p| p.name == name)
    }
    fn mutable(&self, state: &State, name: &str) -> bool {
        state.locals.get(name).is_some_and(|l| l.mutable)
    }
    fn check_reads(&self, toks: &[Token], state: &State) -> Result<(), Diagnostic> {
        for (i,t) in toks.iter().enumerate() {
            if let Tok::Ident(name) = &t.tok {
                if i > 0 && matches!(toks[i-1].tok, Tok::Dot) { continue; }
                if state.moved.contains(name) {
                    return Err(Diagnostic::new(t.pos, "E_MOVE",
                        format!("use of moved actor-owned value '{name}'")));
                }
                if state.loans.contains_key(name) {
                    return Err(Diagnostic::new(t.pos, "E_ESCAPE",
                        "borrowed alias may not escape into an unproved expression"));
                }
                if state.loans.values().any(|loan| loan.exclusive && loan.source == *name) {
                    return Err(Diagnostic::new(t.pos, "E_BORROW",
                        "exclusive actor-owned loan prevents direct source access"));
                }
            }
        }
        Ok(())
    }
    fn check_safepoint(&self, toks: &[Token], state: &State) -> Result<(), Diagnostic> {
        if state.loans.is_empty() { return Ok(()); }
        let unsafe_effect = toks.iter().enumerate().any(|(i,t)|
            ["await","yield","select","spawn","throw","raise","panic",
             "trap","recover","defer"].iter().any(|s| named(t,s))
            || matches!(t.tok, Tok::YieldStar | Tok::Question)
            || is_call(toks,i));
        if unsafe_effect {
            return Err(Diagnostic::new(toks.first().map_or(Pos::default(),|t|t.pos),
                "E_BORROW",
                "active receiver borrow crosses an unproved call, suspension or unwind"));
        }
        Ok(())
    }
    fn require_available(&self, state: &State, source: &str, pos: Pos)
        -> Result<(), Diagnostic> {
        if state.moved.contains(source) {
            return Err(Diagnostic::new(pos,"E_MOVE",
                format!("use of moved actor-owned value '{source}'")));
        }
        if !self.known(state,source) {
            return Err(Diagnostic::new(pos,"E_UNBOUND",
                format!("unknown ownership source '{source}'")));
        }
        if state.loans.contains_key(source) {
            return Err(Diagnostic::new(pos,"E_BORROW",
                "a borrow alias cannot itself be moved or reborrowed without typed lifetimes"));
        }
        Ok(())
    }
    fn borrow(&mut self, state: &mut State, scope: usize,
        dest: String, source: String, exclusive: bool, pos: Pos)
        -> Result<(), Diagnostic> {
        self.require_available(state, &source, pos)?;
        if exclusive && !self.mutable(state,&source) {
            return Err(Diagnostic::new(pos,"E_MUTABILITY",
                "exclusive borrow requires a mutable local source"));
        }
        if state.loans.values().any(|l| l.source == source
            && (exclusive || l.exclusive)) {
            return Err(Diagnostic::new(pos,"E_BORROW",
                "conflicting actor-owned shared/exclusive borrows"));
        }
        if state.locals.contains_key(&dest)
            || self.receiver.params.iter().any(|p| p.name == dest) {
            return Err(Diagnostic::new(pos,"E_DUPLICATE",
                "borrow alias shadows an existing receiver binding"));
        }
        state.locals.insert(dest.clone(),Local{scope,mutable:false});
        state.loans.insert(dest,Loan{source,exclusive,scope});
        self.loans += 1;
        Ok(())
    }
    fn take(&mut self, state: &mut State, source: &str, pos: Pos)
        -> Result<(), Diagnostic> {
        self.require_available(state, source, pos)?;
        if state.loans.values().any(|l| l.source == source) {
            return Err(Diagnostic::new(pos,"E_BORROW",
                "cannot move an actor-owned value while it is borrowed"));
        }
        state.moved.insert(source.to_owned());
        self.moves += 1;
        Ok(())
    }
    fn scan_unknown(&mut self, state: &State)
        -> Result<Vec<Token>, Diagnostic> {
        let start = self.i;
        let mut expected = Vec::<Tok>::new();
        let mut has_brace = false;
        while self.i < self.toks.len() {
            match self.toks[self.i].tok {
                Tok::RBrace if expected.is_empty() => break,
                Tok::Semi if expected.is_empty() => { self.i += 1; break; }
                Tok::LParen => expected.push(Tok::RParen),
                Tok::LBracket => expected.push(Tok::RBracket),
                Tok::LBrace => { expected.push(Tok::RBrace);has_brace=true; },
                Tok::RParen | Tok::RBracket | Tok::RBrace => {
                    if expected.pop().as_ref() != Some(&self.toks[self.i].tok) {
                        return Err(self.err("E_SYNTAX",
                            "mismatched opaque ownership statement delimiter"));
                    }
                    self.i += 1;
                    if expected.is_empty() && has_brace { break; }
                    continue;
                }
                _ => {}
            }
            if expected.len()>128 {
                return Err(self.err("E_NESTING", "ownership statement nesting limit"));
            }
            self.i += 1;
        }
        if !expected.is_empty() {
            return Err(self.err("E_SYNTAX",
                "unterminated opaque ownership statement"));
        }
        let slice=self.toks[start..self.i].to_vec();
        self.check_reads(&slice,state)?;
        self.check_safepoint(&slice,state)?;
        Ok(slice)
    }
    fn statement(&mut self, state: &mut State, scope: usize)
        -> Result<Flow, Diagnostic> {
        let start=self.i;
        if word(self.toks,start,"return")
            || word(self.toks,start,"throw")
            || word(self.toks,start,"raise")
            || word(self.toks,start,"panic") {
            let mut delimiters=Vec::<Tok>::new();
            self.i+=1;
            let begin=self.i;
            loop {
                let Some(t)=self.toks.get(self.i) else {
                    return Err(self.err("E_SYNTAX","unterminated receiver exit"));
                };
                match t.tok {
                    Tok::Semi if delimiters.is_empty() => break,
                    Tok::RBrace if delimiters.is_empty() =>
                        return Err(self.err("E_SYNTAX","missing ';' after receiver exit")),
                    Tok::LParen => delimiters.push(Tok::RParen),
                    Tok::LBracket => delimiters.push(Tok::RBracket),
                    Tok::LBrace => delimiters.push(Tok::RBrace),
                    Tok::RParen | Tok::RBracket | Tok::RBrace => {
                        if delimiters.pop().as_ref()!=Some(&t.tok) {
                            return Err(self.err("E_SYNTAX","mismatched receiver exit"));
                        }
                    }
                    _ => {}
                }
                if delimiters.len()>128 {
                    return Err(self.err("E_NESTING","receiver exit nesting limit"));
                }
                self.i+=1;
            }
            let args=&self.toks[begin..self.i];
            // Escaping aliases get the precise diagnostic before the general
            // active-loan exit/cleanup restriction.
            self.check_reads(args,state)?;
            self.check_safepoint(args,state)?;
            if !state.loans.is_empty() {
                return Err(Diagnostic::new(self.toks[start].pos,"E_BORROW",
                    "receiver exits with active borrows before cleanup is proven"));
            }
            self.i+=1;
            return Ok(Flow {falls_through:false,complete:true});
        }
        // Supported explicit ownership statement:
        // val/let alias = rt borrow [mut] source;
        // val/let dest = rt take source; or rt take source;
        if (word(self.toks,start,"val") || word(self.toks,start,"let"))
            && matches!(self.toks.get(start+1).map(|t| &t.tok),Some(Tok::Ident(_)))
            && matches!(self.toks.get(start+2).map(|t| &t.tok),Some(Tok::Assign))
            && word(self.toks,start+3,"rt") {
            let Tok::Ident(dest)=self.toks[start+1].tok.clone() else {unreachable!()};
            let action=start+4;
            let pos=self.toks[action].pos;
            let is_borrow=word(self.toks,action,"borrow");
            let is_take=word(self.toks,action,"take");
            if is_borrow || is_take {
                let ex=is_borrow && word(self.toks,action+1,"mut");
                let source_idx=action+1+usize::from(ex);
                if let (Some(Token{tok:Tok::Ident(source),..}),
                    Some(Token{tok:Tok::Semi,..}))=
                    (self.toks.get(source_idx),self.toks.get(source_idx+1)) {
                    let source=source.clone();
                    if is_borrow {
                        self.borrow(state,scope,dest,source,ex,pos)?;
                    } else {
                        self.take(state,&source,pos)?;
                        if state.locals.contains_key(&dest)
                            || self.receiver.params.iter().any(|p|p.name==dest) {
                            return Err(Diagnostic::new(pos,"E_DUPLICATE",
                                "ownership destination shadows a receiver binding"));
                        }
                        state.locals.insert(dest,Local{scope,mutable:word(self.toks,start,"let")});
                    }
                    self.i=source_idx+2;
                    return Ok(Flow {falls_through:true,complete:true});
                }
            }
        }
        if word(self.toks,start,"rt") && word(self.toks,start+1,"take") {
            if let (Some(Token{tok:Tok::Ident(source),pos}),
                Some(Token{tok:Tok::Semi,..}))=
                (self.toks.get(start+2),self.toks.get(start+3)) {
                let name=source.clone();
                let location=*pos;
                self.take(state,&name,location)?;
                self.i=start+4;
                return Ok(Flow {falls_through:true,complete:true});
            }
        }
        // Track simple owned local declarations even when the initializer
        // remains unsupported (so ownership can refer to a real local).
        if (word(self.toks,start,"val") || word(self.toks,start,"let"))
            && matches!(self.toks.get(start+1).map(|t| &t.tok),Some(Tok::Ident(_))) {
            let Tok::Ident(name)=self.toks[start+1].tok.clone() else {unreachable!()};
            if state.locals.contains_key(&name)
                || self.receiver.params.iter().any(|p|p.name==name) {
                return Err(Diagnostic::new(self.toks[start+1].pos,"E_DUPLICATE",
                    "receiver binding shadows an owned local or parameter"));
            }
            // Evaluate before the destination is introduced.
            self.i=start+2;
            let value=self.scan_unknown(state)?;
            if value.is_empty() {
                return Err(self.err("E_SYNTAX","invalid receiver binding"));
            }
            state.locals.insert(name,Local {
                scope,mutable:word(self.toks,start,"let"),
            });
            return Ok(Flow {falls_through:true,complete:false});
        }
        let statement=self.scan_unknown(state)?;
        if statement.is_empty() {
            return Err(self.err("E_SYNTAX","receiver statement did not advance"));
        }
        Ok(Flow {falls_through:true,complete:false})
    }
    fn condition(&mut self, state: &State) -> Result<bool, Diagnostic> {
        let start=self.i;
        let mut expected=Vec::<Tok>::new();
        while self.i<self.toks.len() {
            let tok=&self.toks[self.i].tok;
            match tok {
                Tok::LBrace if expected.is_empty() => break,
                Tok::LParen => expected.push(Tok::RParen),
                Tok::LBracket => expected.push(Tok::RBracket),
                Tok::RParen | Tok::RBracket => {
                    if expected.pop().as_ref()!=Some(tok) {
                        return Err(self.err("E_SYNTAX","invalid receiver branch condition"));
                    }
                }
                Tok::RBrace | Tok::Semi if expected.is_empty() =>
                    return Ok(false),
                _ => {}
            }
            if expected.len()>128 {
                return Err(self.err("E_NESTING","receiver condition nesting limit"));
            }
            self.i+=1;
        }
        if !matches!(self.toks.get(self.i).map(|t|&t.tok),Some(Tok::LBrace)) {
            return Ok(false);
        }
        let slice=&self.toks[start..self.i];
        self.check_reads(slice,state)?;
        self.check_safepoint(slice,state)?;
        Ok(true)
    }
    fn if_chain(&mut self, state: &mut State, scope: usize, depth: usize)
        -> Result<Flow, Diagnostic> {
        self.i+=1; // if
        if !self.condition(state)? {
            // Opaque, unsupported branch syntax. It is never a proof.
            let _=self.scan_unknown(state)?;
            return Ok(Flow {falls_through:true,complete:false});
        }
        let starting=state.clone();
        let mut left=starting.clone();
        let yes=self.block(&mut left,scope+1,depth+1)?;
        let mut right=starting;
        let no=if word(self.toks,self.i,"else") {
            self.i+=1;
            if word(self.toks,self.i,"if") {
                self.if_chain(&mut right,scope+1,depth+1)?
            } else if matches!(self.toks.get(self.i).map(|t|&t.tok),Some(Tok::LBrace)) {
                self.block(&mut right,scope+1,depth+1)?
            } else {
                let _=self.scan_unknown(&right)?;
                Flow {falls_through:true,complete:false}
            }
        } else {Flow {falls_through:true,complete:true}};
        self.joins+=1;
        match (yes.falls_through,no.falls_through) {
            (true,true) => {
                left.moved.extend(right.moved);
                *state=left;
            }
            (true,false)=> *state=left,
            (false,true)=> *state=right,
            (false,false)=> {}
        }
        Ok(Flow {
            falls_through:yes.falls_through||no.falls_through,
            complete:yes.complete&&no.complete,
        })
    }
    fn block(&mut self, state:&mut State,scope:usize,depth:usize)
        -> Result<Flow, Diagnostic> {
        if depth>128 {return Err(self.err("E_NESTING","ownership CFG nesting limit"))}
        if !matches!(self.toks.get(self.i).map(|t|&t.tok),Some(Tok::LBrace)) {
            return Err(self.err("E_SYNTAX","expected ownership CFG block"));
        }
        self.i+=1;
        let mut flow=Flow {falls_through:true,complete:true};
        while self.i<self.toks.len()
            && !matches!(self.toks[self.i].tok,Tok::RBrace) {
            let old=self.i;
            let step=if word(self.toks,self.i,"if") {
                self.if_chain(state,scope,depth)?
            } else if matches!(self.toks[self.i].tok,Tok::LBrace) {
                self.block(state,scope+1,depth+1)?
            } else {
                self.statement(state,scope)?
            };
            if self.i==old {return Err(self.err("E_SYNTAX","ownership CFG failed to advance"))}
            flow.complete &= step.complete;
            if flow.falls_through {flow.falls_through=step.falls_through;}
        }
        if !matches!(self.toks.get(self.i).map(|t|&t.tok),Some(Tok::RBrace)) {
            return Err(self.err("E_SYNTAX","unclosed ownership CFG block"));
        }
        self.i+=1;
        let old=state.loans.len();
        state.loans.retain(|_,l|l.scope!=scope);
        self.releases+=old-state.loans.len();
        let removed:Vec<String>=state.locals.iter()
            .filter(|(_,local)|local.scope==scope).map(|(n,_)|n.clone()).collect();
        for name in removed {
            state.locals.remove(&name);
            state.moved.remove(&name);
        }
        Ok(flow)
    }
}

/// Preserve path-sensitive move joins and lexical loan boundaries for the
/// recognized subset, but never claim complete actor-borrow verification.
pub fn check_receiver_borrows(module:&TypedActorModule,body:&ReceiverBodyTokens)
    -> Result<ReceiverBorrowFacts,Diagnostic> {
    let receiver=module.receivers.get(body.receiver)
        .ok_or_else(||Diagnostic::new(Pos::default(),"E_TYPE",
            "invalid receiver ownership index"))?;
    let mut reader=Reader {
        toks:&body.tokens,i:0,receiver,
        moves:0,loans:0,joins:0,releases:0,
    };
    let mut state=State::default();
    let flow=reader.block(&mut state,0,0)?;
    if reader.i!=reader.toks.len() {
        return Err(reader.err("E_SYNTAX","trailing ownership CFG input"));
    }
    if !state.loans.is_empty() {
        return Err(Diagnostic::new(receiver.pos,"E_BORROW",
            "receiver exit with an active borrowed alias"));
    }
    Ok(ReceiverBorrowFacts {
        receiver:body.receiver,moves:reader.moves,loans:reader.loans,
        branch_joins:reader.joins,lexical_loan_releases:reader.releases,
        ownership_preflight_complete:flow.complete,
        normal_fallthrough:flow.falls_through,
    })
}
