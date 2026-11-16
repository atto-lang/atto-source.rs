//! Bounded, syntax-only structural shorthand preflight for future checked lowering.
//! It does not compile/execute structural types, projections or named calls.
use std::collections::HashSet;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{lex, Tok, Token};
type R<T> = Result<T, Diagnostic>;
const LIMIT: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeUse { pub canonical: String, pub structural: bool }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formal { pub name: String, pub ty: TypeUse }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgKind { Ordinary, StructuralProjection, RecordAggregate, MapAggregate }
#[derive(Clone, Debug)]
pub struct Argument { pub label: Option<String>, pub kind: ArgKind, pub value: Vec<Token> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern { pub ty: TypeUse, pub binder: Option<String>, pub guarded: bool }

struct Cursor { ts: Vec<Token>, i: usize }
impl Cursor {
    fn new(src: &str) -> R<Self> { Ok(Self { ts: lex(src)?, i: 0 }) }
    fn tok(&self) -> &Tok { &self.ts[self.i.min(self.ts.len()-1)].tok }
    fn next(&self) -> &Tok { &self.ts[(self.i+1).min(self.ts.len()-1)].tok }
    fn pos(&self) -> Pos { self.ts[self.i.min(self.ts.len()-1)].pos }
    fn err(&self, code: &'static str, msg: &str) -> Diagnostic { Diagnostic::new(self.pos(), code, msg) }
    fn eat(&mut self, t: Tok) -> bool { if self.tok() == &t { self.i += 1; true } else { false } }
    fn word(&self) -> Option<&str> { if let Tok::Ident(s) = self.tok() { Some(s) } else { None } }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.word() == Some(w) { self.i += 1; true } else { false }
    }
    fn ident(&mut self) -> R<String> {
        if let Tok::Ident(s) = self.tok().clone() { self.i += 1; Ok(s) }
        else { Err(self.err("E_SYNTAX", "expected identifier")) }
    }
    fn expect(&mut self, t: Tok) -> R<()> {
        if self.eat(t) { Ok(()) } else { Err(self.err("E_SYNTAX", "unexpected delimiter")) }
    }
    fn finish(&self) -> R<()> {
        if self.tok() == &Tok::Eof { Ok(()) }
        else { Err(self.err("E_SYNTAX", "unexpected trailing token")) }
    }
    fn ty(&mut self, depth: usize) -> R<TypeUse> {
        if depth >= LIMIT { return Err(self.err("E_NESTING", "type nesting limit")); }
        let structural = self.eat(Tok::Tilde) || self.eat_word("structural");
        let mut name = if self.eat(Tok::LBrace) {
            if !structural { return Err(self.err("E_SYNTAX", "shape requires structural")); }
            let mut fields = Vec::new();
            let mut seen = HashSet::new();
            loop {
                if fields.len() >= LIMIT { return Err(self.err("E_SOURCE_LIMIT", "too many fields")); }
                let field = self.ident()?;
                if !seen.insert(field.clone()) { return Err(self.err("E_DUPLICATE", "duplicate structural field")); }
                self.expect(Tok::Colon)?;
                let ty = self.ty(depth+1)?;
                fields.push(format!("{field}:{}", ty.canonical));
                if !self.eat(Tok::Comma) { break; }
            }
            self.expect(Tok::RBrace)?;
            format!("{{{}}}", fields.join(","))
        } else {
            let mut name = self.ident()?;
            while self.eat(Tok::Dot) || self.eat(Tok::ColonColon) {
                name.push('.');
                name.push_str(&self.ident()?);
            }
            if self.eat(Tok::Lt) {
                let mut args = Vec::new();
                loop {
                    if args.len() >= LIMIT { return Err(self.err("E_SOURCE_LIMIT", "too many generic args")); }
                    args.push(self.ty(depth+1)?.canonical);
                    if !self.eat(Tok::Comma) { break; }
                }
                self.expect(Tok::Gt)?;
                name.push('<'); name.push_str(&args.join(",")); name.push('>');
            }
            name
        };
        if structural { name.insert(0, '~'); }
        Ok(TypeUse { canonical: name, structural })
    }
    fn expr(&mut self) -> R<Vec<Token>> {
        let start = self.i;
        let mut closers = Vec::new();
        loop {
            match self.tok() {
                Tok::Eof => return Err(self.err("E_SYNTAX", "unclosed argument")),
                Tok::Comma | Tok::RParen if closers.is_empty() => break,
                Tok::LParen => closers.push(Tok::RParen),
                Tok::LBrace => closers.push(Tok::RBrace),
                Tok::LBracket => closers.push(Tok::RBracket),
                Tok::RParen | Tok::RBrace | Tok::RBracket => {
                    if closers.pop() != Some(self.tok().clone()) {
                        return Err(self.err("E_SYNTAX", "unbalanced argument delimiter"));
                    }
                }
                _ => {}
            }
            if closers.len() > LIMIT { return Err(self.err("E_NESTING", "argument nesting limit")); }
            self.i += 1;
        }
        if self.i == start { return Err(self.err("E_SYNTAX", "missing expression")); }
        Ok(self.ts[start..self.i].to_vec())
    }
}

pub fn parse_formals(src: &str) -> R<Vec<Formal>> {
    let mut c = Cursor::new(src)?;
    c.expect(Tok::LParen)?;
    let mut out: Vec<Formal> = Vec::new();
    if !c.eat(Tok::RParen) {
        loop {
            if out.len() >= LIMIT { return Err(c.err("E_SOURCE_LIMIT", "too many formals")); }
            let (name, ty) = if matches!(c.tok(), Tok::Ident(_)) && c.next() == &Tok::Colon {
                let name = c.ident()?; c.expect(Tok::Colon)?; (name, c.ty(0)?)
            } else { let ty = c.ty(0)?; (c.ident()?, ty) };
            if out.iter().any(|p| p.name == name) { return Err(c.err("E_DUPLICATE", "duplicate formal")); }
            out.push(Formal { name, ty });
            if !c.eat(Tok::Comma) { break; }
            if c.tok() == &Tok::RParen { return Err(c.err("E_SYNTAX", "trailing comma")); }
        }
        c.expect(Tok::RParen)?;
    }
    c.finish()?;
    Ok(out)
}

fn classify(expr: &[Token]) -> R<ArgKind> {
    let first = &expr[0];
    let tilde = first.tok == Tok::Tilde;
    if tilde && expr.get(1).is_some_and(|t| t.tok == Tok::LBrace) {
        return Err(Diagnostic::new(first.pos, "E_SYNTAX", "structural shape is a type, not a value"));
    }
    if tilde && expr.get(1).is_some_and(|t| t.tok == Tok::LParen) {
        if expr.get(2).is_none_or(|t| t.tok == Tok::RParen) {
            return Err(Diagnostic::new(first.pos, "E_SYNTAX", "ambiguous empty aggregate"));
        }
        let mut depth = 0usize;
        let (mut map, mut record) = (false, false);
        for t in expr.iter().skip(2) {
            match t.tok {
                Tok::LParen | Tok::LBrace | Tok::LBracket => depth += 1,
                Tok::RParen if depth == 0 => break,
                Tok::RParen | Tok::RBrace | Tok::RBracket => depth = depth.saturating_sub(1),
                Tok::FatArrow if depth == 0 => map = true,
                Tok::Colon if depth == 0 => record = true,
                _ => {}
            }
        }
        if map && !record { return Ok(ArgKind::MapAggregate); }
        if record && !map && expr.get(2).is_some_and(|t| matches!(t.tok, Tok::Ident(_)))
            && expr.get(3).is_some_and(|t| t.tok == Tok::Colon) {
            return Ok(ArgKind::RecordAggregate);
        }
        return Err(Diagnostic::new(first.pos, "E_SYNTAX", "invalid tilde aggregate; use ~name for structural projection"));
    }
    if tilde || matches!(&first.tok, Tok::Ident(w) if w == "structural") {
        if expr.len() == 1 { return Err(Diagnostic::new(first.pos, "E_SYNTAX", "missing structural operand")); }
        return Ok(ArgKind::StructuralProjection);
    }
    Ok(ArgKind::Ordinary)
}

pub fn parse_arguments(src: &str) -> R<Vec<Argument>> {
    let mut c = Cursor::new(src)?;
    c.expect(Tok::LParen)?;
    let mut out = Vec::new();
    let mut seen_named = false;
    let mut labels = HashSet::new();
    if !c.eat(Tok::RParen) {
        loop {
            if out.len() >= LIMIT { return Err(c.err("E_SOURCE_LIMIT", "too many arguments")); }
            let label = if matches!(c.tok(), Tok::Ident(_)) && c.next() == &Tok::Colon {
                let name = c.ident()?; c.expect(Tok::Colon)?;
                if !labels.insert(name.clone()) { return Err(c.err("E_DUPLICATE", "duplicate argument label")); }
                seen_named = true; Some(name)
            } else {
                if seen_named { return Err(c.err("E_SYNTAX", "positional after named argument")); }
                None
            };
            let value = c.expr()?;
            let kind = classify(&value)?;
            out.push(Argument { label, kind, value });
            if !c.eat(Tok::Comma) { break; }
            if c.tok() == &Tok::RParen { return Err(c.err("E_SYNTAX", "trailing comma")); }
        }
        c.expect(Tok::RParen)?;
    }
    c.finish()?;
    Ok(out)
}

/// Output slots remain in lexical evaluation order; the eventual emitter must
/// evaluate expressions first, and permute temporary values second.
pub fn bind_resolved(formals: &[Formal], args: &[Argument]) -> R<Vec<usize>> {
    let mut filled = HashSet::new();
    let mut next_positional = 0;
    let mut result = Vec::new();
    for arg in args {
        let slot = if let Some(label) = &arg.label {
            formals.iter().position(|f| &f.name == label)
                .ok_or_else(|| Diagnostic::new(Pos::default(), "E_UNBOUND", "unknown label"))?
        } else { let i = next_positional; next_positional += 1; i };
        if slot >= formals.len() { return Err(Diagnostic::new(Pos::default(), "E_ARITY", "extra argument")); }
        if !filled.insert(slot) { return Err(Diagnostic::new(Pos::default(), "E_DUPLICATE", "parameter bound twice")); }
        result.push(slot);
    }
    if filled.len() != formals.len() { return Err(Diagnostic::new(Pos::default(), "E_ARITY", "missing argument")); }
    Ok(result)
}

pub fn parse_pattern(src: &str) -> R<Pattern> {
    let mut c = Cursor::new(src)?;
    c.eat_word("on"); c.eat_word("is");
    let ty = c.ty(0)?;
    if !ty.structural { return Err(c.err("E_SYNTAX", "expected structural type pattern")); }
    let binder = if c.eat_word("as") {
        if !(c.eat_word("val") || c.eat_word("let")) {
            return Err(c.err("E_SYNTAX", "expected val/let binder"));
        }
        Some(c.ident()?)
    } else { None };
    let guarded = c.eat_word("when");
    if guarded {
        if c.tok() == &Tok::Eof { return Err(c.err("E_SYNTAX", "missing guard")); }
        // A target-only guard is retained as syntax. Full expression parsing,
        // purity and ownership validation belong to production match lowering.
        c.i = c.ts.len()-1;
    }
    c.finish()?;
    Ok(Pattern { ty, binder, guarded })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn keyword_and_tilde_have_identical_formal_ast() {
        assert_eq!(parse_formals("(~Point p, count: int)").unwrap(),
                   parse_formals("(structural Point p, count: int)").unwrap());
        assert_eq!(parse_formals("(p: ~Point)").unwrap(),
                   parse_formals("(p: structural Point)").unwrap());
        assert_eq!(parse_formals("(~{x: int, y: bool} p)").unwrap(),
                   parse_formals("(structural {x: int, y: bool} p)").unwrap());
        assert!(parse_formals("(int a, a: bool)").is_err());
        assert!(parse_formals("(~{x: int, x: bool} p)").is_err());
        assert!(parse_formals("(~Point p,)").is_err());
    }
    #[test] fn pattern_alias_and_guard_syntax() {
        assert_eq!(parse_pattern("on is ~Point as val p").unwrap(),
                   parse_pattern("on is structural Point as val p").unwrap());
        assert!(parse_pattern("on ~{x: int} as let p when p.x > 0").unwrap().guarded);
        assert!(parse_pattern("on Point").is_err());
        assert!(parse_pattern("on ~").is_err());
    }
    #[test] fn positional_named_binding_preserves_source_order() {
        let formals = parse_formals("(~Point p, int count, bool verbose)").unwrap();
        let args = parse_arguments("(~p, verbose: true, count: plus(1, 2))").unwrap();
        assert_eq!(bind_resolved(&formals, &args).unwrap(), vec![0, 2, 1]);
        assert_eq!(args[0].kind, ArgKind::StructuralProjection);
        assert!(parse_arguments("(p: 1, 2)").is_err());
        assert!(parse_arguments("(p: 1, p: 2)").is_err());
        assert!(bind_resolved(&formals, &parse_arguments("(1, p: 2, verbose: true)").unwrap()).is_err());
        assert!(bind_resolved(&formals, &parse_arguments("(p: 1, count: 2)").unwrap()).is_err());
        assert!(bind_resolved(&formals, &parse_arguments("(p: 1, count: 2, unknown: 3)").unwrap()).is_err());
    }
    #[test] fn record_map_aggregates_are_not_structural_projections() {
        assert_eq!(parse_arguments("(arg: ~(x: 1))").unwrap()[0].kind, ArgKind::RecordAggregate);
        assert_eq!(parse_arguments("(arg: ~(k => 1))").unwrap()[0].kind, ArgKind::MapAggregate);
        assert_eq!(parse_arguments("(arg: ~point)").unwrap()[0].kind, ArgKind::StructuralProjection);
        assert_eq!(parse_arguments("(arg: structural point)").unwrap()[0].kind, ArgKind::StructuralProjection);
        for bad in ["(~())", "(~ (point))", "(~(x: 1, y => 2))", "(~{x: int})"] {
            assert!(parse_arguments(bad).is_err(), "{bad}");
        }
        let args = parse_arguments("(a: p ? x : y, b: nested(u: 1, v: 2))").unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].label.as_deref(), Some("a"));
    }
}
