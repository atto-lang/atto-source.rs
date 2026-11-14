//! Bounded parse-only grammar for proposed actor declarations and mtd receivers.
//! This does not lower, link, typecheck method bodies, or emit actor Wasm.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::parser::{is_reserved, MAX_FUNCS, MAX_PARAMS};
type R<T> = Result<T, Diagnostic>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverHeader {
    pub owner: String,
    pub name: String,
    pub arity: usize,
    pub mutable: bool,
    pub public: bool,
    pub pos: Pos,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetHeaders {
    pub namespace: Option<String>,
    pub actors: Vec<String>,
    pub receivers: Vec<ReceiverHeader>,
}
struct Cursor<'a> { toks: &'a [Token], i: usize }
impl Cursor<'_> {
    fn tok(&self) -> &Tok { &self.toks[self.i.min(self.toks.len()-1)].tok }
    fn pos(&self) -> Pos { self.toks[self.i.min(self.toks.len()-1)].pos }
    fn word(&self) -> Option<&str> {
        match self.tok() { Tok::Ident(s) => Some(s), _ => None }
    }
    fn next(&mut self) { if self.i + 1 < self.toks.len() { self.i += 1; } }
    fn take(&mut self, t: &Tok) -> bool {
        if self.tok() == t { self.next(); true } else { false }
    }
    fn take_word(&mut self, w: &str) -> bool {
        if self.word() == Some(w) { self.next(); true } else { false }
    }
    fn err(&self, code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(self.pos(), code, message)
    }
    fn expect(&mut self, t: &Tok, message: &str) -> R<()> {
        if self.take(t) { Ok(()) } else { Err(self.err("E_SYNTAX", message)) }
    }
    fn expect_word(&mut self, w: &str) -> R<()> {
        if self.take_word(w) { Ok(()) }
        else { Err(self.err("E_SYNTAX", format!("expected '{w}'"))) }
    }
    fn ident(&mut self) -> R<String> {
        match self.tok().clone() {
            Tok::Ident(s) if !is_reserved(&s)
                && !matches!(s.as_str(), "declare" | "mtd" | "use" | "pub") => {
                self.next(); Ok(s)
            }
            _ => Err(self.err("E_SYNTAX", "expected an identifier")),
        }
    }
    fn type_name(&mut self) -> R<()> {
        match self.tok() {
            Tok::Ident(_) => { self.next(); Ok(()) }
            _ => Err(self.err("E_SYNTAX", "expected a type name")),
        }
    }
    fn params(&mut self) -> R<usize> {
        self.expect(&Tok::LParen, "expected '(' before parameters")?;
        let mut n = 0;
        if self.take(&Tok::RParen) { return Ok(n); }
        loop {
            if n >= MAX_PARAMS { return Err(self.err("E_SOURCE_LIMIT", "too many parameters")); }
            self.type_name()?;
            self.ident()?;
            n += 1;
            if !self.take(&Tok::Comma) {
                self.expect(&Tok::RParen, "expected ')' after parameters")?;
                return Ok(n);
            }
        }
    }
    fn body(&mut self) -> R<()> {
        self.expect(&Tok::LBrace, "expected '{' for a method body")?;
        let mut depth = 1u32;
        while depth != 0 {
            match self.tok() {
                Tok::Eof => return Err(self.err("E_EOF", "unclosed method body")),
                Tok::LBrace => {
                    depth += 1;
                    if depth > 128 { return Err(self.err("E_NESTING", "body nesting limit")); }
                }
                Tok::RBrace => depth -= 1,
                _ => {}
            }
            self.next();
        }
        Ok(())
    }
    fn receiver(&mut self, pos: Pos, public: bool) -> R<ReceiverHeader> {
        self.expect(&Tok::LParen, "expected '(' before receiver")?;
        let mutable = self.take_word("mut");
        self.expect_word("self")?;
        self.expect(&Tok::Colon, "expected ':' after self")?;
        let owner = self.ident()?;
        self.expect(&Tok::RParen, "expected ')' after receiver")?;
        let name = self.ident()?;
        let arity = self.params()?;
        if self.take(&Tok::Colon) || self.take(&Tok::Arrow) { self.type_name()?; }
        self.body()?;
        Ok(ReceiverHeader { owner, name, arity, mutable, public, pos })
    }
    fn function(&mut self) -> R<()> {
        self.ident()?;
        self.params()?;
        if self.take(&Tok::Colon) || self.take(&Tok::Arrow) { self.type_name()?; }
        self.body()
    }
}

/// Validate a target-only source unit: optional flat namespace header,
/// forward-declared actors, out-of-line typed receiver methods and functions.
/// Body braces are balanced only: there is no semantic/state access proof.
pub fn parse_target_declarations(toks: &[Token]) -> R<TargetHeaders> {
    if toks.is_empty() {
        return Err(Diagnostic::new(Pos::default(), "E_EOF", "empty token stream"));
    }
    let mut c = Cursor { toks, i: 0 };
    let mut out = TargetHeaders::default();
    let mut decls = 0;
    while *c.tok() != Tok::Eof {
        if decls >= MAX_FUNCS {
            return Err(c.err("E_SOURCE_LIMIT", "too many target declarations"));
        }
        let pos = c.pos();
        if c.take_word("use") {
            if decls != 0 { return Err(c.err("E_SYNTAX", "namespace header must come first")); }
            c.expect_word("namespace")?;
            out.namespace = Some(c.ident()?);
            c.expect(&Tok::Semi, "expected ';' after namespace")?;
        } else if c.take_word("declare") {
            c.expect_word("actor")?;
            let name = c.ident()?;
            if out.actors.contains(&name) {
                return Err(c.err("E_DUPLICATE", format!("duplicate actor '{name}'")));
            }
            out.actors.push(name);
            c.expect(&Tok::Semi, "expected ';' after actor")?;
        } else {
            let public = c.take_word("pub");
            if c.take_word("mtd") {
                let method = c.receiver(pos, public)?;
                if out.receivers.iter().any(|m: &ReceiverHeader| m.owner == method.owner
                    && m.name == method.name && m.arity == method.arity) {
                    return Err(Diagnostic::new(pos, "E_DUPLICATE",
                        "duplicate receiver signature (owner/name/arity)"));
                }
                out.receivers.push(method);
            } else if c.take_word("fnc") {
                c.function()?;
            } else {
                return Err(c.err("E_SYNTAX",
                    "expected namespace header, declared actor, receiver method, or fnc"));
            }
        }
        decls += 1;
    }
    for receiver in &out.receivers {
        if !out.actors.contains(&receiver.owner) {
            return Err(Diagnostic::new(receiver.pos, "E_UNBOUND",
                format!("receiver owner '{}' is not declared in this source namespace",
                    receiver.owner)));
        }
    }
    Ok(out)
}
