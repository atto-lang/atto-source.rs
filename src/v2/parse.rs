//! Recursive-descent parser of the general ("v2") pipeline.
//!
//! Grammar follows the Java reference (`docs/grammar.ebnf`) for the constructs
//! shared with the legacy parser and adds the async, pattern-matching,
//! closure and actor-composition forms. Depth is bounded so the compiler's
//! own Wasm stack cannot be exhausted by hostile nesting.

use super::ast::*;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::parser::describe;

type R<T> = Result<T, Diagnostic>;

pub const MAX_DEPTH: u32 = 96;
pub const MAX_ITEMS: usize = 1024;

const KEYWORDS: &[&str] = &[
    "define", "fnc", "routine", "suspends", "return", "self", "pub", "const", "val", "let", "mut", "if", "then", "else",
    "elif", "elseif", "fi", "do", "done", "end", "while", "for", "loop", "block", "break", "continue", "match", "over",
    "on", "default", "when", "spawn", "await", "nb", "try", "readch", "writech", "eq", "neq", "is", "new", "true",
    "false", "task", "using", "with", "use", "yield", "pipe", "as", "in",
];

fn unsupported(pos: Pos, what: &str) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", format!("{what} is not supported by the browser compiler yet"))
}

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    depth: u32,
    /// > 0 while parsing an if/while condition, where `,` is AND and `|` is OR
    cond: u32,
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Parser { toks, i: 0, depth: 0, cond: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }
    fn pos(&self) -> Pos {
        self.toks[self.i].pos
    }
    fn advance(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn word_at(&self, n: usize) -> Option<&str> {
        match self.peek_at(n) {
            Tok::Ident(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn is_word(&self, w: &str) -> bool {
        self.word_at(0) == Some(w)
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.advance();
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn expected(&self, what: &str) -> Diagnostic {
        let code = if *self.peek() == Tok::Eof { "E_EOF" } else { "E_SYNTAX" };
        Diagnostic::new(self.pos(), code, format!("expected {what}, found {}", describe(self.peek())))
    }
    fn expect(&mut self, t: Tok, what: &str) -> R<()> {
        if self.eat(&t) {
            Ok(())
        } else {
            Err(self.expected(what))
        }
    }
    fn expect_word(&mut self, w: &str) -> R<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            Err(self.expected(&format!("'{w}'")))
        }
    }
    fn enter(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "nesting is too deep"));
        }
        Ok(())
    }
    fn leave(&mut self) {
        self.depth -= 1;
    }
    fn ident(&mut self, what: &str) -> R<(String, Pos)> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => {
                self.advance();
                Ok((s, pos))
            }
            Tok::Ident(s) => Err(Diagnostic::new(pos, "E_RESERVED", format!("'{s}' is a reserved word; expected {what}"))),
            _ => Err(self.expected(what)),
        }
    }

    // ------------------------------------------------------------- program

    pub fn parse_program(&mut self) -> R<Program> {
        let mut p = Program::default();
        while *self.peek() != Tok::Eof {
            if p.funcs.len() + p.objects.len() + p.uses.len() >= MAX_ITEMS {
                return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "too many top-level declarations"));
            }
            if self.is_word("use") {
                let pos = self.pos();
                self.advance();
                let mut path = vec![self.ident("a module name")?.0];
                while self.eat(&Tok::Dot) {
                    path.push(self.ident("a module name")?.0);
                }
                self.terminator()?;
                p.uses.push((path, pos));
            } else if self.is_word("define") || (self.is_word("pub") && self.word_at(1) == Some("define")) {
                self.eat_word("pub");
                p.objects.push(self.parse_define()?);
            } else {
                p.funcs.push(self.parse_func(false)?);
            }
        }
        Ok(p)
    }

    fn parse_define(&mut self) -> R<Object> {
        self.expect_word("define")?;
        let kind = if self.eat_word("actor") {
            ObjKind::Actor
        } else if self.eat_word("supervisor") {
            ObjKind::Supervisor
        } else {
            let w = self.word_at(0).unwrap_or("declaration").to_owned();
            return Err(unsupported(self.pos(), &format!("'define {w}' together with async/pattern features")));
        };
        let (name, pos) = self.ident("a type name")?;
        self.expect_word("as")?;
        let mut o = Object { kind, name, pos, fields: Vec::new(), receive: None, methods: Vec::new() };
        loop {
            if self.eat_word("end") {
                break;
            }
            if *self.peek() == Tok::Eof {
                return Err(self.expected("'end'"));
            }
            if o.fields.len() + o.methods.len() >= 256 {
                return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "too many members"));
            }
            if self.is_word("state") && *self.peek_at(1) == Tok::LBrace {
                self.advance();
                self.advance();
                while !self.eat(&Tok::RBrace) {
                    o.fields.push(self.parse_field()?);
                }
                continue;
            }
            if self.is_word("receive") && *self.peek_at(1) == Tok::LParen {
                if kind != ObjKind::Actor {
                    return Err(Diagnostic::new(self.pos(), "E_SYNTAX", "only an actor declares 'receive'"));
                }
                if o.receive.is_some() {
                    return Err(Diagnostic::new(self.pos(), "E_DUPLICATE", "an actor declares one 'receive' handler"));
                }
                o.receive = Some(self.parse_receive()?);
                continue;
            }
            if matches!(self.word_at(0), Some("let" | "val" | "const")) {
                o.fields.push(self.parse_field()?);
                continue;
            }
            o.methods.push(self.parse_func(true)?);
        }
        Ok(o)
    }

    fn parse_field(&mut self) -> R<Field> {
        let pos = self.pos();
        let s = self.parse_binding()?;
        self.terminator()?;
        match s {
            Stmt::Bind { kind, target: BindTarget::Name(name), ty, init, .. } => {
                Ok(Field { name, pos, ty, mutable: kind == BindKind::Let, init })
            }
            _ => Err(Diagnostic::new(pos, "E_SYNTAX", "a field declares exactly one name")),
        }
    }

    fn parse_receive(&mut self) -> R<Receive> {
        let pos = self.pos();
        self.advance();
        self.expect(Tok::LParen, "'('")?;
        if !self.is_word("ActorMail") {
            return Err(Diagnostic::new(self.pos(), "E_SYNTAX", "receive takes one 'ActorMail<T> name' parameter"));
        }
        self.advance();
        self.expect(Tok::Lt, "'<'")?;
        let payload = self.parse_type()?;
        self.expect(Tok::Gt, "'>'")?;
        let (mail, _) = self.ident("a parameter name")?;
        self.expect(Tok::RParen, "')'")?;
        if self.eat(&Tok::Colon) || self.eat(&Tok::Arrow) {
            let t = self.parse_type()?;
            if !matches!(&t, TyEx::Name(n, a, _) if n == "void" && a.is_empty()) {
                return Err(Diagnostic::new(t.pos(), "E_TYPE", "receive returns void"));
            }
        }
        let body = self.parse_body()?;
        Ok(Receive { pos, mail, payload, body })
    }

    /// `[pub] [suspends] fnc [*gen] name(params) [: T | -> T] body`. A method
    /// may omit `fnc`.
    fn parse_func(&mut self, method: bool) -> R<Func> {
        self.eat_word("pub");
        let suspends = self.eat_word("suspends");
        let has_kw = self.eat_word("fnc") || self.eat_word("routine") || (method && self.eat_word("mtd"));
        if !has_kw && !method {
            return Err(Diagnostic::new(
                self.pos(),
                "E_TOPLEVEL",
                format!("expected a 'fnc' declaration, found {}", describe(self.peek())),
            ));
        }
        let is_gen = if *self.peek() == Tok::Star {
            self.advance();
            self.expect_word("gen")?;
            true
        } else {
            false
        };
        let (name, pos) = self.ident("a function name")?;
        if *self.peek() == Tok::Lt {
            return Err(unsupported(self.pos(), "generic functions"));
        }
        self.expect(Tok::LParen, "'('")?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                params.push(self.parse_param()?);
                if params.len() > 32 {
                    return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "too many parameters"));
                }
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "')'")?;
        let ret = if self.eat(&Tok::Colon) || self.eat(&Tok::Arrow) {
            let t = self.parse_type()?;
            if matches!(&t, TyEx::Name(n, a, _) if n == "void" && a.is_empty()) {
                None
            } else {
                Some(t)
            }
        } else {
            None
        };
        let body = self.parse_body()?;
        Ok(Func { name, pos, suspends, is_gen, params, ret, body })
    }

    /// `{ ... }`, or an end-delimited body (`:: ... end` or newline `... end`).
    fn parse_body(&mut self) -> R<Vec<Stmt>> {
        if *self.peek() == Tok::LBrace {
            return self.parse_block();
        }
        self.eat(&Tok::ColonColon);
        let body = self.parse_stmts_until(&["end"])?;
        self.expect_word("end")?;
        Ok(body)
    }

    fn parse_param(&mut self) -> R<Param> {
        let pos = self.pos();
        if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
            let (name, pos) = self.ident("a parameter name")?;
            self.advance();
            let ty = self.parse_type()?;
            let mutable = self.eat_word("mut");
            return Ok(Param { name, ty, mutable, pos });
        }
        let ty = self.parse_type()?;
        let mutable = self.eat_word("mut");
        let (name, _) = self.ident("a parameter name")?;
        Ok(Param { name, ty, mutable, pos })
    }

    fn parse_type(&mut self) -> R<TyEx> {
        self.enter()?;
        let pos = self.pos();
        let r = (|| {
            if self.eat(&Tok::LParen) {
                let mut items = vec![self.parse_type()?];
                while self.eat(&Tok::Comma) {
                    items.push(self.parse_type()?);
                }
                self.expect(Tok::RParen, "')'")?;
                if items.len() < 2 || items.len() > 8 {
                    return Err(Diagnostic::new(pos, "E_TYPE", "a tuple type has 2 to 8 elements"));
                }
                return Ok(TyEx::Tuple(items, pos));
            }
            let name = match self.peek().clone() {
                Tok::Ident(s) => {
                    self.advance();
                    s
                }
                _ => return Err(self.expected("a type")),
            };
            let mut args = Vec::new();
            if self.eat(&Tok::Lt) {
                loop {
                    args.push(self.parse_type()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::Gt, "'>'")?;
            }
            Ok(TyEx::Name(name, args, pos))
        })();
        self.leave();
        r
    }

    // ----------------------------------------------------------- statements

    fn parse_block(&mut self) -> R<Vec<Stmt>> {
        self.expect(Tok::LBrace, "'{'")?;
        self.enter()?;
        let mut out = Vec::new();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(self.expected("'}'"));
            }
            out.push(self.parse_stmt()?);
        }
        self.advance();
        self.leave();
        Ok(out)
    }

    fn parse_stmts_until(&mut self, stops: &[&str]) -> R<Vec<Stmt>> {
        self.enter()?;
        let mut out = Vec::new();
        loop {
            if let Some(w) = self.word_at(0) {
                if stops.contains(&w) {
                    break;
                }
            }
            if matches!(self.peek(), Tok::Eof | Tok::RBrace) {
                return Err(self.expected(&format!("'{}'", stops.last().unwrap())));
            }
            out.push(self.parse_stmt()?);
        }
        self.leave();
        Ok(out)
    }

    fn parse_loop_body(&mut self) -> R<Vec<Stmt>> {
        if *self.peek() == Tok::LBrace {
            self.parse_block()
        } else if self.eat_word("do") {
            let body = self.parse_stmts_until(&["done"])?;
            self.expect_word("done")?;
            Ok(body)
        } else {
            Err(self.expected("'do' or '{'"))
        }
    }

    /// A `;`, a statement boundary, or an implicit newline terminator.
    fn terminator(&mut self) -> R<()> {
        if self.eat(&Tok::Semi) {
            return Ok(());
        }
        let boundary = matches!(self.peek(), Tok::RBrace | Tok::Eof)
            || matches!(self.word_at(0), Some("fi" | "end" | "done" | "elif" | "elseif" | "else" | "on" | "default"));
        let newline = self.i > 0 && self.toks[self.i - 1].pos.line < self.pos().line;
        if boundary || newline {
            Ok(())
        } else {
            Err(self.expected("';'"))
        }
    }

    fn parse_stmt(&mut self) -> R<Stmt> {
        self.enter()?;
        let r = self.parse_stmt_inner();
        self.leave();
        r
    }

    fn parse_stmt_inner(&mut self) -> R<Stmt> {
        let pos = self.pos();
        if *self.peek() == Tok::YieldStar {
            self.advance();
            let e = self.parse_expr()?;
            self.terminator()?;
            return Ok(Stmt::YieldFrom(e, pos));
        }
        let word = self.word_at(0).map(str::to_owned);
        match word.as_deref() {
            Some("val" | "const" | "let") => {
                let s = self.parse_binding()?;
                self.terminator()?;
                Ok(s)
            }
            Some("int" | "bool" | "string" | "String")
                if matches!(self.peek_at(1), Tok::Ident(_)) && *self.peek_at(2) == Tok::Assign =>
            {
                let ty = self.parse_type()?;
                let (name, _) = self.ident("a variable name")?;
                self.advance();
                let init = self.parse_expr()?;
                self.terminator()?;
                Ok(Stmt::Bind { kind: BindKind::Const, target: BindTarget::Name(name), ty: Some(ty), init, pos })
            }
            Some("yield") => {
                self.advance();
                let e = self.parse_expr()?;
                self.terminator()?;
                Ok(Stmt::Yield(e, pos))
            }
            Some("return") => {
                self.advance();
                let at_end = matches!(self.peek(), Tok::Semi | Tok::RBrace)
                    || matches!(self.word_at(0), Some("fi" | "end" | "done" | "elif" | "elseif" | "else"));
                let value = if at_end { None } else { Some(self.parse_expr()?) };
                self.terminator()?;
                Ok(Stmt::Return(value, pos))
            }
            Some("if") => self.parse_if(),
            Some("while") => {
                self.advance();
                let cond = self.parse_cond()?;
                self.eat(&Tok::Semi);
                let body = self.parse_loop_body()?;
                Ok(Stmt::While { cond, body, pos })
            }
            Some("loop") => {
                self.advance();
                let body = self.parse_loop_body()?;
                Ok(Stmt::Loop { body, pos })
            }
            Some("for") => self.parse_for(),
            Some("block") => {
                self.advance();
                Ok(Stmt::Block(self.parse_block()?))
            }
            Some("break") => {
                self.advance();
                self.terminator()?;
                Ok(Stmt::Break(pos))
            }
            Some("continue") => {
                self.advance();
                self.terminator()?;
                Ok(Stmt::Continue(pos))
            }
            Some("match") => {
                let e = self.parse_expr()?;
                self.eat(&Tok::Semi);
                Ok(Stmt::Expr(e))
            }
            Some("writech") => {
                self.advance();
                let ch = self.parse_unary()?;
                self.expect(Tok::Comma, "',' after the channel")?;
                let value = self.parse_expr()?;
                self.terminator()?;
                Ok(Stmt::WriteCh { ch, value, pos })
            }
            Some("do") if self.word_at(1) == Some("task") => {
                self.advance();
                self.advance();
                let body = self.parse_block()?;
                self.terminator()?;
                Ok(Stmt::DoTask(body, pos))
            }
            Some("using") => {
                self.advance();
                let resource = self.parse_expr()?;
                let name = if self.eat_word("as") { Some(self.ident("a binding name")?.0) } else { None };
                let body = self.parse_block()?;
                Ok(Stmt::Using { resource, name, body, pos })
            }
            Some("with") => {
                self.advance();
                let scope = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::With { scope, body, pos })
            }
            Some(w @ ("throw" | "try" | "switch" | "select" | "defer" | "raise" | "panic"))
                if !(w == "try" && self.word_at(1) == Some("readch")) =>
            {
                Err(unsupported(pos, &format!("'{w}' together with async/pattern features")))
            }
            Some("fnc" | "routine") => Err(unsupported(pos, "nested functions")),
            _ => {
                let e = self.parse_expr()?;
                if *self.peek() == Tok::PlusAssign {
                    let at = self.pos();
                    self.advance();
                    let value = self.parse_expr()?;
                    self.terminator()?;
                    let sum = |target: Expr| Expr { kind: Ex::Binary(BinOp::Add, Box::new(target), Box::new(value)), pos: at };
                    return match e.kind.clone() {
                        Ex::Var(name) => Ok(Stmt::Assign { name, value: sum(e), pos }),
                        Ex::Member(target, field) => Ok(Stmt::FieldAssign { target: *target, field, value: sum(e), pos }),
                        _ => Err(Diagnostic::new(at, "E_ASSIGN_TARGET", "'+=' needs a variable or field")),
                    };
                }
                if *self.peek() == Tok::Assign {
                    let at = self.pos();
                    self.advance();
                    let value = self.parse_expr()?;
                    self.terminator()?;
                    return match e.kind {
                        Ex::Var(name) => Ok(Stmt::Assign { name, value, pos }),
                        Ex::Member(target, field) => Ok(Stmt::FieldAssign { target: *target, field, value, pos }),
                        _ => Err(Diagnostic::new(at, "E_ASSIGN_TARGET", "only a variable or a field can be assigned")),
                    };
                }
                if matches!(self.peek(), Tok::PlusPlus | Tok::MinusMinus) {
                    return Err(Diagnostic::new(
                        self.pos(),
                        "E_UNSUPPORTED",
                        "'++' and '--' exist only in a for update clause",
                    ));
                }
                self.terminator()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    /// `("val"|"const"|"let") (name [":" type] | type name | "(" names ")") "=" expression`
    fn parse_binding(&mut self) -> R<Stmt> {
        let pos = self.pos();
        let kind = if self.eat_word("val") {
            BindKind::Val
        } else if self.eat_word("const") {
            BindKind::Const
        } else {
            self.expect_word("let")?;
            BindKind::Let
        };
        self.eat_word("mut");
        let (target, ty) = if *self.peek() == Tok::LParen {
            self.advance();
            let mut names = vec![self.ident("a variable name")?.0];
            while self.eat(&Tok::Comma) {
                names.push(self.ident("a variable name")?.0);
            }
            self.expect(Tok::RParen, "')'")?;
            (BindTarget::Tuple(names), None)
        } else if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
            let (name, _) = self.ident("a variable name")?;
            self.advance();
            (BindTarget::Name(name), Some(self.parse_type()?))
        } else if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Ident(_) | Tok::Lt) {
            let ty = self.parse_type()?;
            let (name, _) = self.ident("a variable name")?;
            (BindTarget::Name(name), Some(ty))
        } else {
            (BindTarget::Name(self.ident("a variable name")?.0), None)
        };
        if *self.peek() != Tok::Assign {
            return Err(Diagnostic::new(self.pos(), "E_UNINITIALIZED", "a binding needs an initializer"));
        }
        self.advance();
        let init = self.parse_expr()?;
        Ok(Stmt::Bind { kind, target, ty, init, pos })
    }

    fn parse_if(&mut self) -> R<Stmt> {
        let pos = self.pos();
        self.expect_word("if")?;
        let cond = self.parse_cond()?;
        let mut arms = Vec::new();
        let mut otherwise = None;
        let elif = |p: &mut Self| {
            p.eat_word("elif") || p.eat_word("elseif") || (p.is_word("else") && p.word_at(1) == Some("if") && {
                p.advance();
                p.advance();
                true
            })
        };
        if *self.peek() == Tok::LBrace {
            arms.push((cond, self.parse_block()?));
            loop {
                if elif(self) {
                    let c = self.parse_cond()?;
                    arms.push((c, self.parse_block()?));
                } else if self.eat_word("else") {
                    otherwise = Some(self.parse_block()?);
                    break;
                } else {
                    break;
                }
                if arms.len() > 64 {
                    return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "too many elif arms"));
                }
            }
            self.eat_word("fi");
            return Ok(Stmt::If { arms, otherwise, pos });
        }
        self.eat(&Tok::Semi);
        if !(self.eat_word("then") || self.eat_word("do")) {
            return Err(self.expected("'then'"));
        }
        let stops = ["elif", "elseif", "else", "fi"];
        arms.push((cond, self.parse_stmts_until(&stops)?));
        loop {
            if elif(self) {
                let c = self.parse_cond()?;
                self.eat(&Tok::Semi);
                if !(self.eat_word("then") || self.eat_word("do")) {
                    return Err(self.expected("'then'"));
                }
                arms.push((c, self.parse_stmts_until(&stops)?));
            } else if self.eat_word("else") {
                otherwise = Some(self.parse_stmts_until(&["fi"])?);
                break;
            } else {
                break;
            }
            if arms.len() > 64 {
                return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "too many elif arms"));
            }
        }
        if !self.eat_word("fi") {
            return Err(Diagnostic::new(
                self.pos(),
                if *self.peek() == Tok::Eof { "E_EOF" } else { "E_FI" },
                format!("expected 'fi' to close the if started at {}:{}, found {}", pos.line, pos.col, describe(self.peek())),
            ));
        }
        Ok(Stmt::If { arms, otherwise, pos })
    }

    fn parse_for(&mut self) -> R<Stmt> {
        let pos = self.pos();
        self.expect_word("for")?;
        let is_await = self.is_word("await") && matches!(self.peek_at(1), Tok::Ident(_)) && self.word_at(2) == Some("in");
        if is_await {
            self.advance();
        }
        if matches!(self.peek(), Tok::Ident(_)) && self.word_at(1) == Some("in") {
            let (name, _) = self.ident("an iteration variable")?;
            self.expect_word("in")?;
            let source = self.parse_expr()?;
            let body = self.parse_loop_body()?;
            return Ok(Stmt::ForIn { name, source, body, is_await, pos });
        }
        let paren = self.eat(&Tok::LParen);
        let init = if *self.peek() == Tok::Semi {
            None
        } else if matches!(self.word_at(0), Some("val" | "const" | "let")) {
            Some(Box::new(self.parse_binding()?))
        } else {
            let pos = self.pos();
            let (name, _) = self.ident("a loop variable")?;
            self.expect(Tok::Assign, "'='")?;
            let value = self.parse_expr()?;
            Some(Box::new(Stmt::Assign { name, value, pos }))
        };
        self.expect(Tok::Semi, "';'")?;
        let cond = if *self.peek() == Tok::Semi {
            None
        } else {
            self.cond += 1;
            let c = self.parse_cond_or();
            self.cond -= 1;
            Some(c?)
        };
        self.expect(Tok::Semi, "';'")?;
        let update_end = if paren { Tok::RParen } else { Tok::LBrace };
        let update = if *self.peek() == update_end || (!paren && self.is_word("do")) {
            None
        } else {
            let upos = self.pos();
            let (name, _) = self.ident("a loop variable")?;
            let step = |op: BinOp, name: String| Stmt::Assign {
                value: Expr {
                    kind: Ex::Binary(
                        op,
                        Box::new(Expr { kind: Ex::Var(name.clone()), pos: upos }),
                        Box::new(Expr { kind: Ex::Int(Some(1)), pos: upos }),
                    ),
                    pos: upos,
                },
                name,
                pos: upos,
            };
            if self.eat(&Tok::PlusPlus) {
                Some(Box::new(step(BinOp::Add, name)))
            } else if self.eat(&Tok::MinusMinus) {
                Some(Box::new(step(BinOp::Sub, name)))
            } else if self.eat(&Tok::PlusAssign) {
                let by = self.parse_expr()?;
                Some(Box::new(Stmt::Assign {
                    value: Expr {
                        kind: Ex::Binary(BinOp::Add, Box::new(Expr { kind: Ex::Var(name.clone()), pos: upos }), Box::new(by)),
                        pos: upos,
                    },
                    name,
                    pos: upos,
                }))
            } else {
                self.expect(Tok::Assign, "'++', '--' or '='")?;
                let value = self.parse_expr()?;
                Some(Box::new(Stmt::Assign { name, value, pos: upos }))
            }
        };
        if paren {
            self.expect(Tok::RParen, "')'")?;
        }
        let body = self.parse_loop_body()?;
        Ok(Stmt::For { init, cond, update, body, pos })
    }

    // ---------------------------------------------------------- expressions

    /// An if/while condition: `condition_and { "|" condition_and }`.
    fn parse_cond(&mut self) -> R<Expr> {
        self.enter()?;
        self.cond += 1;
        let r = self.parse_cond_or();
        self.cond -= 1;
        self.leave();
        let c = r?;
        if *self.peek() == Tok::Assign {
            return Err(Diagnostic::new(
                self.pos(),
                "E_ASSIGN_CONDITION",
                "assignment is not allowed in a condition; use 'eq' to compare",
            ));
        }
        Ok(c)
    }

    fn parse_cond_or(&mut self) -> R<Expr> {
        let mut left = self.parse_cond_and()?;
        while *self.peek() == Tok::Pipe {
            let pos = self.pos();
            self.advance();
            let right = self.parse_cond_and()?;
            left = Expr { kind: Ex::Binary(BinOp::CondOr, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    fn parse_cond_and(&mut self) -> R<Expr> {
        let mut left = self.parse_or()?;
        while *self.peek() == Tok::Comma {
            let pos = self.pos();
            self.advance();
            let right = self.parse_or()?;
            left = Expr { kind: Ex::Binary(BinOp::CondAnd, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    pub fn parse_expr(&mut self) -> R<Expr> {
        self.enter()?;
        let saved = self.cond;
        self.cond = 0;
        let r = self.parse_ternary();
        self.cond = saved;
        self.leave();
        r
    }

    fn parse_ternary(&mut self) -> R<Expr> {
        let mut c = self.parse_actor_pipe()?;
        // Functional pipeline: value |> f |> g(extra) is g(f(value), extra).
        while *self.peek() == Tok::PipeGt {
            let pos = self.pos();
            self.advance();
            let stage = self.parse_postfix()?;
            let (name, targs, mut args) = match stage.kind {
                Ex::Var(name) => (name, Vec::new(), Vec::new()),
                Ex::Call { name, targs, args } => (name, targs, args),
                _ => return Err(unsupported(pos, "pipeline stages other than named function calls")),
            };
            args.insert(0, Arg { name: None, value: c });
            c = Expr { kind: Ex::Call { name, targs, args }, pos };
        }
        if *self.peek() == Tok::Question {
            let pos = self.pos();
            self.advance();
            let a = self.parse_expr()?;
            self.expect(Tok::Colon, "':'")?;
            self.enter()?;
            let b = self.parse_ternary();
            self.leave();
            return Ok(Expr { kind: Ex::Ternary(Box::new(c), Box::new(a), Box::new(b?)), pos });
        }
        Ok(c)
    }

    /// `a | b` and `a pipe b` link actor references. Inside a condition `|`
    /// is the eager boolean OR instead.
    fn parse_actor_pipe(&mut self) -> R<Expr> {
        let mut left = self.parse_or()?;
        loop {
            let pos = self.pos();
            if (self.cond == 0 && *self.peek() == Tok::Pipe) || self.is_word("pipe") {
                self.advance();
                let right = self.parse_or()?;
                left = Expr { kind: Ex::Binary(BinOp::Pipe, Box::new(left), Box::new(right)), pos };
            } else {
                return Ok(left);
            }
        }
    }

    fn binary_chain(&mut self, next: fn(&mut Self) -> R<Expr>, tok: Tok, op: BinOp) -> R<Expr> {
        let mut left = next(self)?;
        let mut links = 0;
        while *self.peek() == tok {
            let pos = self.pos();
            self.advance();
            links += 1;
            if links > 256 {
                return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "operator chain is too long"));
            }
            let right = next(self)?;
            left = Expr { kind: Ex::Binary(op, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> R<Expr> {
        self.binary_chain(Self::parse_xor, Tok::PipePipe, BinOp::Or)
    }
    fn parse_xor(&mut self) -> R<Expr> {
        self.binary_chain(Self::parse_and, Tok::CaretCaret, BinOp::Xor)
    }
    fn parse_and(&mut self) -> R<Expr> {
        self.binary_chain(Self::parse_equality, Tok::AmpAmp, BinOp::And)
    }

    fn parse_equality(&mut self) -> R<Expr> {
        let mut left = self.parse_comparison()?;
        let mut links = 0;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                Tok::Ident(s) if s == "eq" => BinOp::Eq,
                Tok::Ident(s) if s == "neq" => BinOp::Ne,
                Tok::Bang if self.word_at(1) == Some("eq") => {
                    self.advance();
                    BinOp::Ne
                }
                Tok::Ident(s) if s == "is" => {
                    self.advance();
                    let ty = self.parse_type()?;
                    left = Expr { kind: Ex::Is(Box::new(left), ty), pos };
                    continue;
                }
                _ => break,
            };
            self.advance();
            links += 1;
            if links > 256 {
                return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "operator chain is too long"));
            }
            let right = self.parse_comparison()?;
            left = Expr { kind: Ex::Binary(op, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> R<Expr> {
        let mut left = self.parse_additive()?;
        let mut links = 0;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            links += 1;
            if links > 256 {
                return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "operator chain is too long"));
            }
            let right = self.parse_additive()?;
            left = Expr { kind: Ex::Binary(op, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> R<Expr> {
        let mut left = self.parse_multiplicative()?;
        let mut links = 0;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            links += 1;
            if links > 256 {
                return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "operator chain is too long"));
            }
            let right = self.parse_multiplicative()?;
            left = Expr { kind: Ex::Binary(op, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> R<Expr> {
        let mut left = self.parse_unary()?;
        let mut links = 0;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            links += 1;
            if links > 256 {
                return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "operator chain is too long"));
            }
            let right = self.parse_unary()?;
            left = Expr { kind: Ex::Binary(op, Box::new(left), Box::new(right)), pos };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> R<Expr> {
        self.enter()?;
        let r = self.parse_unary_inner();
        self.leave();
        r
    }

    fn parse_unary_inner(&mut self) -> R<Expr> {
        let pos = self.pos();
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Plus => UnOp::Plus,
            Tok::Bang => UnOp::Not,
            Tok::Ident(s) if s == "await" => {
                self.advance();
                let inner = self.parse_unary()?;
                return Ok(Expr { kind: Ex::Await(Box::new(inner)), pos });
            }
            Tok::Ident(s) if s == "readch" || ((s == "try" || s == "nb") && self.word_at(1) == Some("readch")) => {
                let mode = match s.as_str() {
                    "try" => ReadMode::Try,
                    "nb" => ReadMode::Nb,
                    _ => ReadMode::Wait,
                };
                if mode != ReadMode::Wait {
                    self.advance();
                }
                self.advance();
                let ch = self.parse_unary()?;
                return Ok(Expr { kind: Ex::ReadCh(mode, Box::new(ch)), pos });
            }
            Tok::Ident(s) if s == "trap" => return Err(unsupported(pos, "'trap' together with async/pattern features")),
            _ => return self.parse_postfix(),
        };
        self.advance();
        let inner = self.parse_unary()?;
        Ok(Expr { kind: Ex::Unary(op, Box::new(inner)), pos })
    }

    fn parse_args(&mut self) -> R<Vec<Arg>> {
        self.expect(Tok::LParen, "'('")?;
        let mut args = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                let name = if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
                    let (n, _) = self.ident("an argument name")?;
                    self.advance();
                    Some(n)
                } else {
                    None
                };
                args.push(Arg { name, value: self.parse_expr()? });
                if args.len() > 32 {
                    return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "too many call arguments"));
                }
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "')' to close the call")?;
        Ok(args)
    }

    fn parse_postfix(&mut self) -> R<Expr> {
        let pos = self.pos();
        let mut e = self.parse_primary()?;
        let mut links = 0;
        loop {
            links += 1;
            if links > 256 {
                return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "member chain is too long"));
            }
            match self.peek() {
                Tok::LParen => {
                    let name = match &e.kind {
                        Ex::Var(n) => n.clone(),
                        _ => return Err(unsupported(self.pos(), "calling a computed value")),
                    };
                    let args = self.parse_args()?;
                    e = Expr { kind: Ex::Call { name, targs: Vec::new(), args }, pos };
                }
                Tok::Dot => {
                    let dot = self.pos();
                    self.advance();
                    let member = match self.peek().clone() {
                        Tok::Ident(m) => m,
                        _ => return Err(self.expected("a member name after '.'")),
                    };
                    self.advance();
                    if *self.peek() == Tok::LParen {
                        let args = self.parse_args()?;
                        e = Expr { kind: Ex::Method { base: Box::new(e), name: member, args }, pos: dot };
                    } else {
                        e = Expr { kind: Ex::Member(Box::new(e), member), pos: dot };
                    }
                }
                Tok::LBracket => return Err(unsupported(self.pos(), "indexing")),
                _ => break,
            }
        }
        Ok(e)
    }

    /// `Name<T, ...>` followed by `(` or `.`: explicit type arguments. Only
    /// tried for a capitalised head or the `channel` factory, so `a < b`
    /// stays a comparison.
    fn try_type_args(&mut self) -> Option<Vec<TyEx>> {
        let save = (self.i, self.depth);
        self.advance();
        let mut args = Vec::new();
        loop {
            match self.parse_type() {
                Ok(t) => args.push(t),
                Err(_) => break,
            }
            if self.eat(&Tok::Comma) {
                continue;
            }
            if self.eat(&Tok::Gt) && matches!(self.peek(), Tok::LParen | Tok::Dot) {
                return Some(args);
            }
            break;
        }
        self.i = save.0;
        self.depth = save.1;
        None
    }

    fn parse_primary(&mut self) -> R<Expr> {
        let pos = self.pos();
        let kind = match self.peek().clone() {
            Tok::Int(v) => {
                self.advance();
                Ex::Int(v)
            }
            Tok::Str(s) => {
                self.advance();
                Ex::Str(s)
            }
            Tok::Float(_) => return Err(unsupported(pos, "float values together with async/pattern features")),
            Tok::LParen => {
                self.advance();
                if self.cond > 0 {
                    let inner = self.parse_cond_or()?;
                    self.expect(Tok::RParen, "')'")?;
                    return Ok(Expr { kind: inner.kind, pos });
                }
                let first = self.parse_expr()?;
                if *self.peek() == Tok::Comma {
                    let mut items = vec![first];
                    while self.eat(&Tok::Comma) {
                        items.push(self.parse_expr()?);
                    }
                    self.expect(Tok::RParen, "')'")?;
                    if items.len() > 8 {
                        return Err(Diagnostic::new(pos, "E_TYPE", "a tuple has 2 to 8 elements"));
                    }
                    return Ok(Expr { kind: Ex::Tuple(items), pos });
                }
                self.expect(Tok::RParen, "')'")?;
                return Ok(Expr { kind: first.kind, pos });
            }
            Tok::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if *self.peek() != Tok::RBracket {
                    loop {
                        items.push(self.parse_expr()?);
                        if items.len() > 1024 {
                            return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "list literal is too long"));
                        }
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Tok::RBracket, "']'")?;
                Ex::List(items)
            }
            Tok::Pipe | Tok::PipePipe => return self.parse_lambda(),
            Tok::Ident(s) => match s.as_str() {
                "true" => {
                    self.advance();
                    Ex::Bool(true)
                }
                "false" => {
                    self.advance();
                    Ex::Bool(false)
                }
                "null" => return Err(Diagnostic::new(pos, "E_NULL", "AttoLang has no null value; use Option<T>")),
                "self" => {
                    self.advance();
                    Ex::SelfRef
                }
                "spawn" => {
                    self.advance();
                    let (actor, _) = self.ident("an actor name after 'spawn'")?;
                    let args = self.parse_args()?;
                    Ex::Spawn(actor, args)
                }
                "task" if *self.peek_at(1) == Tok::LBrace => {
                    self.advance();
                    Ex::Task(self.parse_block()?)
                }
                "match" => return self.parse_match(),
                "new" => return Err(unsupported(pos, "classes together with async/pattern features")),
                w if KEYWORDS.contains(&w) => {
                    return Err(Diagnostic::new(pos, "E_SYNTAX", format!("expected an expression, found keyword '{w}'")))
                }
                _ => {
                    let generic_head = s.chars().next().is_some_and(char::is_uppercase) || s == "channel";
                    if *self.peek_at(1) == Tok::Lt && generic_head {
                        self.advance();
                        // positioned on '<'
                        self.i -= 0;
                        if let Some(targs) = self.try_type_args() {
                            if *self.peek() == Tok::LParen {
                                let args = self.parse_args()?;
                                return self.finish_postfix(Expr { kind: Ex::Call { name: s, targs, args }, pos });
                            }
                            self.advance(); // '.'
                            let (name, _) = self.ident("a member name after '.'")?;
                            let args = self.parse_args()?;
                            let ty = TyEx::Name(s, targs, pos);
                            return self.finish_postfix(Expr { kind: Ex::TypeStatic { ty, name, args }, pos });
                        }
                        return Ok(Expr { kind: Ex::Var(s), pos });
                    }
                    self.advance();
                    Ex::Var(s)
                }
            },
            _ => return Err(self.expected("an expression")),
        };
        Ok(Expr { kind, pos })
    }

    /// A generic call or type-static call was consumed inside `parse_primary`;
    /// the caller's postfix loop continues from the returned expression.
    fn finish_postfix(&mut self, e: Expr) -> R<Expr> {
        Ok(e)
    }

    /// `|a, b| -> [T] { body }` (also `|| -> { body }`).
    fn parse_lambda(&mut self) -> R<Expr> {
        let pos = self.pos();
        let mut params = Vec::new();
        if !self.eat(&Tok::PipePipe) {
            self.expect(Tok::Pipe, "'|'")?;
            while *self.peek() != Tok::Pipe {
                let ppos = self.pos();
                if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Comma | Tok::Pipe) {
                    let (name, _) = self.ident("a parameter name")?;
                    params.push(LambdaParam { name, ty: None, pos: ppos });
                } else if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
                    let (name, _) = self.ident("a parameter name")?;
                    self.advance();
                    params.push(LambdaParam { name, ty: Some(self.parse_type()?), pos: ppos });
                } else {
                    let ty = self.parse_type()?;
                    let (name, _) = self.ident("a parameter name")?;
                    params.push(LambdaParam { name, ty: Some(ty), pos: ppos });
                }
                if params.len() > 8 {
                    return Err(Diagnostic::new(ppos, "E_SOURCE_LIMIT", "too many lambda parameters"));
                }
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::Pipe, "'|' to close the lambda parameters")?;
        }
        self.expect(Tok::Arrow, "'->' after the lambda parameters")?;
        let ret = if *self.peek() == Tok::LBrace { None } else { Some(self.parse_type()?) };
        let body = self.parse_block()?;
        Ok(Expr { kind: Ex::Lambda { params, ret, body }, pos })
    }

    /// `match subject over { on P [when g] -> body } end`; a body is a block
    /// or an expression (optionally followed by `,`). `default ->` equals
    /// `on _ ->`.
    fn parse_match(&mut self) -> R<Expr> {
        let pos = self.pos();
        self.expect_word("match")?;
        let subject = self.parse_expr()?;
        self.expect_word("over")?;
        let mut arms = Vec::new();
        loop {
            if self.eat_word("end") {
                break;
            }
            let apos = self.pos();
            if arms.len() >= 256 {
                return Err(Diagnostic::new(apos, "E_SOURCE_LIMIT", "too many match arms"));
            }
            let pat = if self.eat_word("default") {
                Pat::Wild
            } else if self.eat_word("on") {
                self.parse_pattern()?
            } else {
                return Err(Diagnostic::new(
                    apos,
                    if *self.peek() == Tok::Eof { "E_EOF" } else { "E_SYNTAX" },
                    "every match arm starts with 'on' (or 'default'); close the match with 'end'",
                ));
            };
            let guard = if self.eat_word("when") { Some(self.parse_expr()?) } else { None };
            self.expect(Tok::Arrow, "'->' after the match pattern")?;
            let body = if *self.peek() == Tok::LBrace {
                ArmBody::Block(self.parse_block()?)
            } else {
                ArmBody::Expr(self.parse_expr()?)
            };
            self.eat(&Tok::Comma);
            arms.push(Arm { pat, guard, body, pos: apos });
        }
        if arms.is_empty() {
            return Err(Diagnostic::new(pos, "E_SYNTAX", "match requires at least one arm"));
        }
        Ok(Expr { kind: Ex::Match(Box::new(subject), arms), pos })
    }

    fn parse_pattern(&mut self) -> R<Pat> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Int(Some(v)) => {
                self.advance();
                Ok(Pat::Int(v as i64))
            }
            Tok::Minus => {
                self.advance();
                match self.peek().clone() {
                    Tok::Int(Some(v)) => {
                        self.advance();
                        Ok(Pat::Int(-(v as i64)))
                    }
                    Tok::Int(None) => {
                        self.advance();
                        Ok(Pat::Int(i64::MIN))
                    }
                    _ => Err(self.expected("an integer after '-'")),
                }
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Pat::Str(s))
            }
            Tok::Ident(w) => match w.as_str() {
                "true" | "false" => {
                    self.advance();
                    Ok(Pat::Bool(w == "true"))
                }
                "_" => {
                    self.advance();
                    Ok(Pat::Wild)
                }
                "is" => {
                    self.advance();
                    Ok(Pat::Is(self.parse_type()?))
                }
                "None" => {
                    self.advance();
                    Ok(Pat::Variant(w, None))
                }
                "Some" | "Ok" | "Err" => {
                    self.advance();
                    self.expect(Tok::LParen, "'(' after the variant")?;
                    let name = match self.peek().clone() {
                        Tok::Ident(n) if n == "_" => {
                            self.advance();
                            n
                        }
                        _ => self.ident("a payload binding name")?.0,
                    };
                    self.expect(Tok::RParen, "')' after the payload binding")?;
                    Ok(Pat::Variant(w, Some(name)))
                }
                _ => {
                    let (name, _) = self.ident("a match pattern")?;
                    if name.chars().next().is_some_and(char::is_uppercase) {
                        return Err(unsupported(pos, "constructor patterns"));
                    }
                    Ok(Pat::Bind(name))
                }
            },
            _ => Err(Diagnostic::new(pos, "E_SYNTAX", "expected match pattern")),
        }
    }
}
