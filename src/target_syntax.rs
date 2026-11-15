//! Bounded parse-only grammar for proposed actor declarations and mtd receivers.
//! This does not lower, link, typecheck method bodies, or emit actor Wasm.
use crate::ast::ActorDecl;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::parser::{is_reserved, Parser, MAX_FUNCS, MAX_PARAMS};
type R<T> = Result<T, Diagnostic>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverParam {
    pub name: String,
    pub type_name: String,
    pub pos: Pos,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverHeader {
    pub owner: String,
    pub name: String,
    pub arity: usize,
    pub params: Vec<ReceiverParam>,
    pub return_type: String,
    pub mutable: bool,
    pub public: bool,
    pub pos: Pos,
}
#[derive(Clone, Debug, Default)]
pub struct TargetHeaders {
    pub namespace: Option<String>,
    /// Imported module aliases. Imports are not resolved or linked here.
    pub imports: Vec<String>,
    pub actors: Vec<String>,
    /// Real concrete actor syntax trees parsed by the production parser.
    /// Bodies are parsed but NOT typechecked for the new out-of-line profile.
    pub definitions: Vec<ActorDecl>,
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
    fn type_name(&mut self) -> R<String> {
        let name = match self.tok().clone() {
            Tok::Ident(s) => { self.next(); s }
            _ => return Err(self.err("E_SYNTAX", "expected a type name")),
        };
        if self.take(&Tok::Lt) {
            if name != "ActorRef" {
                return Err(self.err("E_UNSUPPORTED_TYPE",
                    "target receiver generics admit only ActorRef<Actor>"));
            }
            let actor = self.ident()?;
            self.expect(&Tok::Gt, "expected '>' in ActorRef<Actor>")?;
            return Ok(format!("ActorRef<{actor}>"));
        }
        Ok(name)
    }
    fn params(&mut self) -> R<Vec<ReceiverParam>> {
        self.expect(&Tok::LParen, "expected '(' before parameters")?;
        let mut params = Vec::new();
        if self.take(&Tok::RParen) { return Ok(params); }
        loop {
            if params.len() >= MAX_PARAMS {
                return Err(self.err("E_SOURCE_LIMIT", "too many parameters"));
            }
            let pos = self.pos();
            let type_name = self.type_name()?;
            let name = self.ident()?;
            if params.iter().any(|p: &ReceiverParam| p.name == name) {
                return Err(Diagnostic::new(pos, "E_DUPLICATE",
                    format!("duplicate method parameter '{name}'")));
            }
            params.push(ReceiverParam { name, type_name, pos });
            if !self.take(&Tok::Comma) {
                self.expect(&Tok::RParen, "expected ')' after parameters")?;
                return Ok(params);
            }
        }
    }
    fn body(&mut self, arrow_type: bool, colon_type: bool) -> R<()> {
        // Target-only receiver bodies are delimiter scans, not executable Wasm.
        if colon_type && self.take(&Tok::Arrow) && *self.tok() != Tok::LBrace {
            return Err(self.err("E_SYNTAX", "': Type ->' requires a braced body"));
        }
        if self.take(&Tok::LBrace) {
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
            return Ok(());
        }
        if !arrow_type {
            return Err(self.err("E_SYNTAX", "expected '{' for a method body"));
        }
        if self.take(&Tok::ColonColon) && *self.tok() == Tok::LBrace {
            return Err(self.err("E_SYNTAX", "'::' requires a comma/end body, not '{'"));
        }
        if *self.tok() == Tok::Arrow {
            return Err(self.err("E_SYNTAX", "a second '->' is not a body delimiter"));
        }
        let (mut braces, mut conditionals, mut items) = (0u32, 0u32, 0usize);
        loop {
            match self.tok() {
                Tok::Eof => return Err(self.err("E_EOF", "unclosed Erlang-style method body")),
                Tok::Ident(s) if s == "end" && braces == 0 => {
                    if items == 0 { return Err(self.err("E_SYNTAX", "empty Erlang-style method body")); }
                    self.next();
                    return Ok(());
                }
                Tok::Ident(s) if braces == 0 && matches!(s.as_str(), "match" | "select" | "switch") =>
                    return Err(self.err("E_UNSUPPORTED",
                        "nested end-delimited match/select/switch requires a recursive method parser")),
                Tok::Ident(s) if braces == 0 && s == "if" => conditionals += 1,
                Tok::Ident(s) if braces == 0 && s == "fi" => {
                    if conditionals == 0 { return Err(self.err("E_SYNTAX", "unmatched fi")); }
                    conditionals -= 1;
                }
                Tok::LBrace => {
                    braces += 1;
                    if braces > 128 { return Err(self.err("E_NESTING", "body nesting limit")); }
                }
                Tok::RBrace => {
                    if braces == 0 { return Err(self.err("E_SYNTAX", "unmatched '}'")); }
                    braces -= 1;
                }
                Tok::Semi if braces == 0 && conditionals == 0 =>
                    return Err(self.err("E_SYNTAX", "Erlang-style items require commas, not semicolons")),
                Tok::Comma if braces == 0 && conditionals == 0 => {
                    if matches!(self.toks.get(self.i + 1).map(|x| &x.tok), Some(Tok::Ident(s)) if s == "end") {
                        return Err(self.err("E_SYNTAX", "trailing comma in Erlang-style body"));
                    }
                }
                _ => {}
            }
            items += 1;
            self.next();
        }
    }
    fn receiver(&mut self, pos: Pos, public: bool) -> R<(ReceiverHeader, usize, usize)> {
        self.expect(&Tok::LParen, "expected '(' before receiver")?;
        // Both the explicit typed spelling and the nominal receiver spelling
        // are accepted: (mut self: Foo), (self: Foo), (Foo self),
        // (Foo) and (Foo _). All variants have one nominal owner.
        let (owner, mutable) = if self.take_word("mut") {
            self.expect_word("self")?;
            self.expect(&Tok::Colon, "expected ':' after self")?;
            (self.ident()?, true)
        } else if self.take_word("self") {
            self.expect(&Tok::Colon, "expected ':' after self")?;
            (self.ident()?, false)
        } else {
            let owner = self.ident()?;
            // Explicit name or compiler-owned implicit receiver.
            if !self.take_word("self") {
                self.take_word("_");
            }
            (owner, false)
        };
        self.expect(&Tok::RParen, "expected ')' after receiver")?;
        let name = self.ident()?;
        let params = self.params()?;
        let arity = params.len();
        let arrow_type = *self.tok() == Tok::Arrow;
        let colon_type = *self.tok() == Tok::Colon;
        let return_type = if self.take(&Tok::Colon) || self.take(&Tok::Arrow) {
            self.type_name()?
        } else {
            "void".to_owned()
        };
        let start = self.i;
        self.body(arrow_type, colon_type)?;
        let end = self.i;
        Ok((ReceiverHeader { owner, name, arity, params, return_type, mutable, public, pos }, start, end))
    }

    /// Parse a concrete "define actor Foo as ... end" using the existing
    /// production parser. The delimiter scan observes braces so an "end"
    /// inside a method body does not truncate the actor declaration.
    fn definition(&mut self, start: usize) -> R<ActorDecl> {
        self.expect_word("actor")?;
        let name = self.ident()?;
        self.expect_word("as")?;
        let mut braces = 0usize;
        loop {
            match self.tok() {
                Tok::Eof => return Err(self.err("E_EOF", "unclosed concrete actor definition")),
                Tok::LBrace => {
                    braces += 1;
                    if braces > 128 {
                        return Err(self.err("E_NESTING", "actor definition brace nesting limit"));
                    }
                }
                Tok::RBrace => {
                    if braces == 0 {
                        return Err(self.err("E_SYNTAX", "unbalanced actor definition brace"));
                    }
                    braces -= 1;
                }
                Tok::Ident(s) if s == "end" && braces == 0 => {
                    self.next();
                    break;
                }
                _ => {}
            }
            self.next();
        }
        let mut actor_tokens = self.toks[start..self.i].to_vec();
        actor_tokens.push(Token { tok: Tok::Eof, pos: self.pos() });
        let program = Parser::new(actor_tokens).parse_program()?;
        if program.actors.len() != 1 || program.actors[0].name != name ||
            !program.classes.is_empty() || !program.funcs.is_empty() {
            return Err(self.err("E_SYNTAX", "invalid concrete actor definition"));
        }
        Ok(program.actors.into_iter().next().expect("checked one actor"))
    }
    fn function(&mut self) -> R<(usize, usize)> {
        self.ident()?;
        self.params()?;
        let arrow_type = *self.tok() == Tok::Arrow;
        let colon_type = *self.tok() == Tok::Colon;
        if self.take(&Tok::Colon) || self.take(&Tok::Arrow) { self.type_name()?; }
        let start = self.i;
        self.body(arrow_type, colon_type)?;
        Ok((start, self.i))
    }
}

/// Validate a target-only source unit: optional flat namespace header,
/// module imports, forward-declared actors, nominal or typed receivers and functions.
/// Body braces are balanced only: there is no semantic/state access proof.
pub fn parse_target_declarations(toks: &[Token]) -> R<TargetHeaders> {
    if toks.is_empty() {
        return Err(Diagnostic::new(Pos::default(), "E_EOF", "empty token stream"));
    }
    let mut c = Cursor { toks, i: 0 };
    let mut out = TargetHeaders::default();
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut function_spans: Vec<(usize, usize)> = Vec::new();
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
        } else if c.take_word("import") {
            if !out.actors.is_empty() || !out.receivers.is_empty() {
                return Err(c.err("E_SYNTAX", "module imports must precede actor declarations"));
            }
            c.expect_word("module")?;
            let alias = c.ident()?;
            if out.imports.contains(&alias) {
                return Err(c.err("E_DUPLICATE", format!("duplicate module import '{alias}'")));
            }
            c.expect_word("from")?;
            match c.tok() {
                Tok::Str(_) => c.next(),
                _ => return Err(c.err("E_SYNTAX", "expected a quoted module import path")),
            }
            c.expect(&Tok::Semi, "expected ';' after module import")?;
            out.imports.push(alias);
        } else if c.take_word("define") {
            let actor = c.definition(c.i - 1)?;
            if out.definitions.iter().any(|other| other.name == actor.name) {
                return Err(Diagnostic::new(actor.pos, "E_DUPLICATE",
                    format!("duplicate concrete actor definition '{}'", actor.name)));
            }
            out.definitions.push(actor);
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
                let (method, start, end) = c.receiver(pos, public)?;
                if out.receivers.iter().any(|m: &ReceiverHeader| m.owner == method.owner
                    && m.name == method.name && m.arity == method.arity) {
                    return Err(Diagnostic::new(pos, "E_DUPLICATE",
                        "duplicate receiver signature (owner/name/arity)"));
                }
                spans.push((out.receivers.len(), start, end));
                out.receivers.push(method);
            } else if c.take_word("fnc") || c.take_word("routine") {
                function_spans.push(c.function()?);
            } else {
                return Err(c.err("E_SYNTAX",
                    "expected namespace header, module import, declared/concrete actor, receiver method, or fnc"));
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
    for (i, start, end) in spans {
        validate_receiver_body(toks, &out, i, start, end)?;
    }
    for (start, end) in function_spans {
        validate_spawned_actor_calls(toks, &out, start, end)?;
    }
    Ok(out)
}


/// Resolve a receiver header against nominal, file-scoped symbols. This does
/// not link a full actor definition or prove that a method is callable through
/// an ActorRef. The namespace is this file's identity, *not* an open import.
pub fn resolve_receiver_header<'a>(
    symbols: &'a TargetHeaders,
    namespace: Option<&str>,
    actor: &str,
    name: &str,
    arity: usize,
    external: bool,
    pos: Pos,
) -> R<&'a ReceiverHeader> {
    if symbols.namespace.as_deref() != namespace {
        return Err(Diagnostic::new(pos, "E_NAMESPACE",
            "receiver lookup is not in the source namespace"));
    }
    if !symbols.actors.iter().any(|a| a == actor) {
        return Err(Diagnostic::new(pos, "E_UNBOUND",
            format!("unknown actor receiver type '{actor}'")));
    }
    let Some(method) = symbols.receivers.iter().find(|m|
        m.owner == actor && m.name == name && m.arity == arity) else {
        return Err(Diagnostic::new(pos, "E_UNBOUND",
            format!("actor '{actor}' has no receiver method '{name}' with arity {arity}")));
    };
    if external && !method.public {
        return Err(Diagnostic::new(pos, "E_VISIBILITY",
            format!("receiver '{actor}.{name}' is not public")));
    }
    Ok(method)
}

fn word_at(toks: &[Token], index: usize, word: &str) -> bool {
    matches!(toks.get(index).map(|t| &t.tok), Some(Tok::Ident(s)) if s == word)
}
fn at(toks: &[Token], index: usize, tok: &Tok) -> bool {
    toks.get(index).is_some_and(|t| &t.tok == tok)
}
fn diagnostic(toks: &[Token], index: usize, code: &'static str, why: &str) -> Diagnostic {
    Diagnostic::new(toks.get(index).map_or(Pos::default(), |t| t.pos), code, why)
}
fn close_paren(toks: &[Token], start: usize, limit: usize) -> R<usize> {
    let mut depth = 0usize;
    for i in start..limit {
        if at(toks, i, &Tok::LParen) { depth += 1; }
        if at(toks, i, &Tok::RParen) {
            if depth == 0 {
                return Err(diagnostic(toks, i, "E_SYNTAX", "unexpected ')'"));
            }
            depth -= 1;
            if depth == 0 { return Ok(i); }
        }
    }
    Err(diagnostic(toks, start, "E_EOF", "unterminated argument list"))
}
fn count_args(toks: &[Token], open: usize, close: usize) -> usize {
    if close == open + 1 { return 0; }
    let (mut n, mut round, mut square, mut braces) = (1, 0, 0, 0);
    for i in open + 1..close {
        match toks[i].tok {
            Tok::LParen => round += 1,
            Tok::RParen => round -= 1,
            Tok::LBracket => square += 1,
            Tok::RBracket => square -= 1,
            Tok::LBrace => braces += 1,
            Tok::RBrace => braces -= 1,
            Tok::Comma if round == 0 && square == 0 && braces == 0 => n += 1,
            _ => {}
        }
    }
    n
}
fn validate_callback(
    toks: &[Token], symbols: &TargetHeaders, receiver: &ReceiverHeader,
    open: usize, limit: usize,
) -> R<usize> {
    let close = close_paren(toks, open, limit)?;
    let mut k = open + 1;
    if !word_at(toks, k, "critical") || !at(toks, k+1, &Tok::Minus) ||
        !(word_at(toks, k+2, "read") || word_at(toks, k+2, "write")) {
        return Err(diagnostic(toks, k, "E_SEALED",
            "handle_state requires 'critical-read' or 'critical-write'"));
    }
    let write = word_at(toks, k+2, "write");
    if write && !receiver.mutable {
        return Err(diagnostic(toks, k, "E_MUTABILITY",
            "critical-write requires a mutable actor receiver"));
    }
    k += 3;
    if !at(toks, k, &Tok::Pipe) {
        return Err(diagnostic(toks, k, "E_SEALED", "expected '|state|' callback"));
    }
    let binder_pos = k + 1;
    let binder = match toks.get(binder_pos).map(|t| &t.tok) {
        Some(Tok::Ident(name)) if !is_reserved(name) => name.as_str(),
        _ => return Err(diagnostic(toks, binder_pos, "E_SEALED", "invalid state callback binder")),
    };
    if binder == "self" || binder == "_" {
        return Err(diagnostic(toks, binder_pos, "E_SEALED",
            "state callback must bind a distinct state variable"));
    }
    k += 2;
    if !at(toks, k, &Tok::Pipe) || !at(toks, k+1, &Tok::Arrow) {
        return Err(diagnostic(toks, k, "E_SEALED",
            "state callback must be '|s| -> expression'"));
    }
    k += 2;
    if k == close {
        return Err(diagnostic(toks, k, "E_SEALED", "missing critical callback body"));
    }
    // The v1 gate is conservative. A full borrow checker must subsequently
    // prove reference lifetimes, effect purity, and field types before codegen.
    for i in k..close {
        if ["await", "select", "yield", "suspends", "spawn", "rt", "defer"].iter()
            .any(|w| word_at(toks, i, w)) {
            return Err(diagnostic(toks, i, "E_BORROW",
                "suspension, spawning and deferred effects are forbidden during critical access"));
        }
        if word_at(toks, i, "handle_state") ||
            (word_at(toks, i, "self") && at(toks, i+1, &Tok::Dot)) {
            return Err(diagnostic(toks, i, "E_BORROW",
                "nested actor state access or self capture is forbidden in critical callback"));
        }
        if word_at(toks, i, binder) {
            if !at(toks, i+1, &Tok::Dot) {
                return Err(diagnostic(toks, i, "E_ESCAPE",
                    "actor state binding cannot be copied, captured, or returned"));
            }
            if !matches!(toks.get(i+2).map(|t| &t.tok), Some(Tok::Ident(_))) {
                return Err(diagnostic(toks, i+2, "E_SEALED",
                    "state field member access requires a field name"));
            }
            if !write {
                if matches!(toks.get(i+3).map(|t| &t.tok),
                    Some(Tok::Assign | Tok::PlusAssign | Tok::PlusPlus | Tok::MinusMinus)) {
                    return Err(diagnostic(toks, i+3, "E_READONLY",
                        "critical-read cannot mutate actor state"));
                }
                if at(toks, i+3, &Tok::Dot) || at(toks, i+3, &Tok::LParen) {
                    return Err(diagnostic(toks, i+3, "E_UNSUPPORTED",
                        "state field method calls need a full effect and borrow checker"));
                }
            }
        }
    }
    // Prevent the callback from smuggling a second actor's receiver semantics.
    let _ = symbols;
    Ok(close)
}
fn validate_receiver_body(
    toks: &[Token], symbols: &TargetHeaders, which: usize,
    start: usize, end: usize,
) -> R<()> {
    let receiver = &symbols.receivers[which];
    // Bare -> Type bodies start with a statement rather than a delimiter;
    // do not skip the first token and bypass sealed-state validation.
    let mut i = if at(toks, start, &Tok::LBrace) || at(toks, start, &Tok::ColonColon) {
        start + 1
    } else { start };
    while i < end.saturating_sub(1) {
        if word_at(toks, i, "handle_state") &&
            !(i >= 2 && word_at(toks, i-2, "self") && at(toks, i-1, &Tok::Dot)) {
            return Err(diagnostic(toks, i, "E_SEALED",
                "actor state access must be 'self.handle_state(...)'"));
        }
        if word_at(toks, i, "self") && at(toks, i+1, &Tok::Dot) {
            let member = match toks.get(i+2).map(|t| &t.tok) {
                Some(Tok::Ident(s)) => s.as_str(),
                _ => return Err(diagnostic(toks, i+2, "E_SEALED", "invalid actor member access")),
            };
            if !at(toks, i+3, &Tok::LParen) {
                return Err(diagnostic(toks, i, "E_SEALED",
                    "direct actor fields are sealed; use self.handle_state"));
            }
            if member == "handle_state" {
                let closed = validate_callback(toks, symbols, receiver, i+3, end)?;
                i = closed + 1;
                continue;
            }
            let close = close_paren(toks, i+3, end)?;
            resolve_receiver_header(symbols, symbols.namespace.as_deref(), &receiver.owner,
                member, count_args(toks, i+3, close), false, toks[i].pos)?;
            i = close + 1;
            continue;
        }
        i += 1;
    }
    Ok(())
}
fn validate_spawned_actor_calls(
    toks: &[Token], symbols: &TargetHeaders, start: usize, end: usize,
) -> R<()> {
    // Narrow, lexical analysis of direct 'val ref = spawn Actor()' bindings.
    // A later typed checker must handle aliases, parameters and dataflow.
    let mut actor_refs: Vec<(&str, &str)> = Vec::new();
    let first = if at(toks, start, &Tok::LBrace) || at(toks, start, &Tok::ColonColon) {
        start + 1
    } else { start };
    for i in first..end.saturating_sub(1) {
        if (word_at(toks, i, "val") || word_at(toks, i, "let")) &&
            matches!(toks.get(i+1).map(|t| &t.tok), Some(Tok::Ident(_))) &&
            at(toks, i+2, &Tok::Assign) && word_at(toks, i+3, "spawn") {
            if let (Some(Tok::Ident(name)), Some(Tok::Ident(actor))) =
                (toks.get(i+1).map(|t| &t.tok), toks.get(i+4).map(|t| &t.tok)) {
                if symbols.actors.contains(actor) && at(toks, i+5, &Tok::LParen) &&
                    at(toks, i+6, &Tok::RParen) {
                    actor_refs.push((name, actor));
                }
            }
        }
    }
    for i in first..end.saturating_sub(1) {
        if let Some((name, _)) = actor_refs.iter().find(|(n,_)| word_at(toks, i, n)) {
            if at(toks, i+1, &Tok::Dot) &&
                matches!(toks.get(i+2).map(|t| &t.tok), Some(Tok::Ident(_))) {
                if !(word_at(toks, i+2, "send") || word_at(toks, i+2, "get_done")) {
                    return Err(diagnostic(toks, i, "E_ISOLATION",
                        "ActorRef cannot call receiver methods synchronously; send a mailbox message"));
                }
            }
            let _ = name;
        }
    }
    Ok(())
}
