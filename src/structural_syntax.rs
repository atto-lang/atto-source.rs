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
pub struct Argument {
    pub label: Option<String>,
    pub kind: ArgKind,
    pub value: Vec<Token>,
    /// The first token of this source argument, including any top-level label.
    /// Diagnostics must not invent a zero/default source location.
    pub pos: Pos,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    /// Requires the runtime subject to implement a reifiable structural type.
    TypeTest(TypeUse),
    /// Name resolved later to an immutable, statically known pattern constant.
    /// A type name alone is *not* a valid constant pattern.
    ValueConstant(String),
    /// Each field independently tracks presence plus the value pattern that
    /// must succeed if present. Subject-only extra fields pass.
    ValueFields(Vec<FieldPattern>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldPattern {
    pub name: String,
    /// The optional-marker permits absence but checks the value if present.
    pub optional: bool,
    pub value: ScalarPattern,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScalarPattern { Any, Int(i64), Bool(bool), Str(String) }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern { pub kind: PatternKind, pub binder: Option<String>, pub guarded: bool }

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
        self.ty_inner(depth, false)
    }

    /// Parse a type-level union independently of conditional-expression '|'.
    /// Union arms are a SET (commutative/idempotent), whereas a structural
    /// row's source field order is not part of its read-only type identity.
    fn ty_inner(&mut self, depth: usize, allow_literal: bool) -> R<TypeUse> {
        if depth >= LIMIT { return Err(self.err("E_NESTING", "type nesting limit")); }
        let mut arms = vec![self.ty_atom(depth, allow_literal)?];
        while self.eat(Tok::Pipe) {
            if arms.len() >= LIMIT {
                return Err(self.err("E_SOURCE_LIMIT", "too many union type variants"));
            }
            arms.push(self.ty_atom(depth, allow_literal)?);
        }
        if arms.len() == 1 { return Ok(arms.remove(0)); }
        // Do not silently erase ~T's borrowing/shape origin when a top-level
        // formal mixes structural and nominal types.
        if depth == 0 && arms.iter().any(|a| a.structural) &&
           arms.iter().any(|a| !a.structural) {
            return Err(self.err("E_UNSUPPORTED_TYPE",
                "mixed nominal/structural union formal needs a checked tagged-view ABI"));
        }
        let structural = arms.iter().all(|a| a.structural);
        let mut names: Vec<String> = arms.into_iter().map(|a| a.canonical).collect();
        names.sort_unstable();
        names.dedup();
        Ok(TypeUse { canonical: names.join("|"), structural })
    }

    fn ty_atom(&mut self, depth: usize, allow_literal: bool) -> R<TypeUse> {
        let explicit = self.eat(Tok::Tilde) || self.eat_word("structural");
        if self.eat(Tok::LBrace) {
            let mut fields = Vec::new();
            let mut seen = HashSet::new();
            if self.tok() == &Tok::RBrace {
                return Err(self.err("E_SYNTAX", "empty structural shape is not a wildcard"));
            }
            loop {
                if fields.len() >= LIMIT {
                    return Err(self.err("E_SOURCE_LIMIT", "too many structural fields"));
                }
                let field = self.ident()?;
                if !seen.insert(field.clone()) {
                    return Err(self.err("E_DUPLICATE", "duplicate structural field"));
                }
                if self.tok() == &Tok::Question {
                    return Err(self.err("E_UNSUPPORTED_TYPE",
                        "optional type-shape fields require checked presence/read semantics"));
                }
                self.expect(Tok::Colon)?;
                let ty = self.ty_inner(depth + 1, true)?;
                fields.push((field, ty.canonical));
                if !self.eat(Tok::Comma) { break; }
                if self.tok() == &Tok::RBrace {
                    return Err(self.err("E_SYNTAX", "trailing structural field comma"));
                }
            }
            self.expect(Tok::RBrace)?;
            fields.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            let fields = fields.into_iter()
                .map(|(key, ty)| format!("{key}:{ty}"))
                .collect::<Vec<_>>().join(",");
            // The ~ prefix is the *canonical* representation for implicit
            // anonymous structural types as well as the two explicit forms.
            return Ok(TypeUse { canonical: format!("~{{{fields}}}"), structural: true });
        }
        let name = match self.tok().clone() {
            Tok::Str(v) if allow_literal && !explicit => {
                self.i += 1;
                format!("{v:?}")
            }
            Tok::Int(Some(v)) if allow_literal && !explicit && v <= i64::MAX as u64 => {
                self.i += 1;
                v.to_string()
            }
            Tok::Minus if allow_literal && !explicit &&
                matches!(self.next(), Tok::Int(Some(_))) => {
                self.i += 1;
                let Tok::Int(Some(v)) = self.tok().clone() else { unreachable!() };
                if v > i64::MAX as u64 + 1 {
                    return Err(self.err("E_INTEGER_RANGE", "singleton integer out of range"));
                }
                self.i += 1;
                if v == i64::MAX as u64 + 1 {
                    i64::MIN.to_string()
                } else {
                    format!("-{v}")
                }
            }
            Tok::Ident(ref w) if allow_literal && !explicit &&
                (w == "true" || w == "false") => {
                self.i += 1;
                w.clone()
            }
            Tok::Ident(_) => {
                let mut name = self.ident()?;
                if name == "_" {
                    return Err(self.err("E_TYPE",
                        "'_' is a value-pattern wildcard, not an implicit Any type"));
                }
                if explicit && matches!(name.as_str(),
                    "int" | "bool" | "string" | "String" | "float" | "void") {
                    return Err(self.err("E_UNSUPPORTED_TYPE",
                        "structural view requires a product shape, not a scalar type"));
                }
                while self.eat(Tok::Dot) || self.eat(Tok::ColonColon) {
                    name.push('.');
                    name.push_str(&self.ident()?);
                }
                if self.eat(Tok::Lt) {
                    let mut args = Vec::new();
                    loop {
                        if args.len() >= LIMIT {
                            return Err(self.err("E_SOURCE_LIMIT", "too many generic arguments"));
                        }
                        args.push(self.ty_inner(depth + 1, false)?.canonical);
                        if !self.eat(Tok::Comma) { break; }
                    }
                    self.expect(Tok::Gt)?;
                    name.push('<');
                    name.push_str(&args.join(","));
                    name.push('>');
                }
                name
            }
            _ => return Err(self.err("E_SYNTAX",
                "expected a field type; singleton literal types belong inside shape fields")),
        };
        Ok(TypeUse {
            canonical: if explicit { format!("~{name}") } else { name },
            structural: explicit,
        })
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

/// Bounded *syntax-only* recognition of the first right-hand operand of
/// a structural projection. The production compiler must still prove the
/// callee's structural formal, public fields, borrow lifetime and adapter ABI.
/// A token prefix alone is NOT evidence that a safe projection is possible.
fn check_projection_operand(expr: &[Token], first: Pos) -> R<()> {
    let Some(Tok::Ident(head)) = expr.get(1).map(|t| &t.tok) else {
        return Err(Diagnostic::new(first, "E_SYNTAX",
            "structural argument requires a postfix value operand"));
    };
    if matches!(head.as_str(), "true" | "false" | "null" | "new"
        | "await" | "spawn" | "is" | "structural" | "fnc" | "self") {
        // Explicit self is possible in method arguments, but until a checked
        // receiver-loan ABI exists, it must not be mistaken for an ordinary
        // owned argument.
        return Err(Diagnostic::new(first, "E_UNSUPPORTED",
            "structural projection of a keyword, literal or receiver is not admitted"));
    }
    let mut i = 2usize;
    while i < expr.len() {
        match &expr[i].tok {
            Tok::Dot | Tok::ColonColon => {
                if !matches!(expr.get(i+1).map(|t| &t.tok), Some(Tok::Ident(_))) {
                    return Err(Diagnostic::new(expr[i].pos, "E_SYNTAX",
                        "structural projection member access requires a name"));
                }
                i += 2;
            }
            Tok::LParen | Tok::LBracket => {
                let mut stack = Vec::new();
                loop {
                    let Some(token) = expr.get(i) else {
                        return Err(Diagnostic::new(first, "E_SYNTAX",
                            "unclosed structural postfix expression"));
                    };
                    match token.tok {
                        Tok::LParen => stack.push(Tok::RParen),
                        Tok::LBracket => stack.push(Tok::RBracket),
                        Tok::LBrace => stack.push(Tok::RBrace),
                        Tok::RParen | Tok::RBracket | Tok::RBrace => {
                            if stack.pop() != Some(token.tok.clone()) {
                                return Err(Diagnostic::new(token.pos, "E_SYNTAX",
                                    "unbalanced structural postfix expression"));
                            }
                        }
                        _ => {}
                    }
                    if stack.len() > LIMIT {
                        return Err(Diagnostic::new(token.pos, "E_NESTING",
                            "structural projection postfix expression is too deep"));
                    }
                    i += 1;
                    if stack.is_empty() { break; }
                }
            }
            _ => return Err(Diagnostic::new(expr[i].pos, "E_UNSUPPORTED",
                "structural argument preflight admits a single postfix operand only; full operators need typed lowering")),
        }
    }
    Ok(())
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
        if matches!(expr[1].tok, Tok::LBrace | Tok::LParen) {
            return Err(Diagnostic::new(first.pos, "E_SYNTAX",
                "structural argument projection needs a value; brace shapes are types and parenthesized aggregates use '~' only"));
        }
        check_projection_operand(expr, first.pos)?;
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
            let arg_pos = c.pos();
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
            out.push(Argument { label, kind, value, pos: arg_pos });
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
                .ok_or_else(|| Diagnostic::new(arg.pos, "E_UNBOUND", format!("unknown named argument '{}'", label)))?
        } else { let i = next_positional; next_positional += 1; i };
        if slot >= formals.len() { return Err(Diagnostic::new(arg.pos, "E_ARITY", "extra argument")); }
        if !filled.insert(slot) { return Err(Diagnostic::new(arg.pos, "E_DUPLICATE", "parameter bound twice")); }
        result.push(slot);
    }
    if filled.len() != formals.len() {
        return Err(Diagnostic::new(args.last().map_or(Pos::default(), |a| a.pos),
            "E_ARITY", "missing argument"));
    }
    Ok(result)
}

/// Source-only binding contract. The returned indices are in SOURCE
/// evaluation order, not declaration order. Later typed lowering must
/// evaluate each value once before permuting temporary slots.
pub fn bind_checked_structural(formals: &[Formal], args: &[Argument]) -> R<Vec<usize>> {
    let slots = bind_resolved(formals, args)?;
    for (arg, &slot) in args.iter().zip(&slots) {
        if arg.kind == ArgKind::StructuralProjection && !formals[slot].ty.structural {
            return Err(Diagnostic::new(arg.pos, "E_TYPE",
                format!("explicit '~' argument requires a structural formal '{}'",
                    formals[slot].name)));
        }
    }
    Ok(slots)
}

pub fn parse_pattern(src: &str) -> R<Pattern> {
    let mut c = Cursor::new(src)?;
    c.eat_word("on");
    let is_type = c.eat_word("is");
    let kind = if is_type {
        let ty = c.ty(0)?;
        if !ty.structural {
            return Err(c.err("E_SYNTAX", "is type test requires ~ or structural"));
        }
        PatternKind::TypeTest(ty)
    } else {
        // Anonymous value patterns are structural by default. 'is' selects
        // a type test; without 'is', field tokens denote VALUE patterns.
        if !(c.eat(Tok::Tilde) || c.eat_word("structural")) && c.tok() != &Tok::LBrace {
            return Err(c.err("E_SYNTAX", "value pattern needs ~, structural or a field shape"));
        }
        if c.eat(Tok::LBrace) {
            // Value patterns are deliberately not the same as structural
            // type shapes. All field values must be literal or '_'.
            // No getters, effects, or arbitrary runtime expressions.
            let mut fields = Vec::new();
            let mut names = HashSet::new();
            if c.tok() == &Tok::RBrace {
                return Err(c.err("E_SYNTAX", "empty field value pattern"));
            }
            loop {
                if fields.len() >= LIMIT {
                    return Err(c.err("E_SOURCE_LIMIT", "too many field value patterns"));
                }
                let name = c.ident()?;
                if !names.insert(name.clone()) {
                    return Err(c.err("E_DUPLICATE", "duplicate value pattern field"));
                }
                // Question modifies field PRESENCE only, not its value test.
                // Delimiters are separate tokens; trivia makes no difference.
                let optional = c.eat(Tok::Question);
                c.expect(Tok::Colon)?;
                let value = match c.tok().clone() {
                    Tok::Ident(w) if w == "_" => {
                        c.i += 1; ScalarPattern::Any
                    }
                    Tok::Ident(w) if w == "true" || w == "false" => {
                        c.i += 1; ScalarPattern::Bool(w == "true")
                    }
                    Tok::Int(Some(n)) if n <= i64::MAX as u64 => {
                        c.i += 1; ScalarPattern::Int(n as i64)
                    }
                    Tok::Minus if matches!(c.next(), Tok::Int(Some(_))) => {
                        c.i += 1;
                        let Tok::Int(Some(magnitude)) = c.tok().clone() else { unreachable!() };
                        let value = if magnitude == i64::MAX as u64 + 1 {
                            i64::MIN
                        } else if magnitude <= i64::MAX as u64 {
                            -(magnitude as i64)
                        } else {
                            return Err(c.err("E_INTEGER_RANGE", "negative pattern integer out of range"));
                        };
                        c.i += 1; ScalarPattern::Int(value)
                    }
                    Tok::Str(s) => { c.i += 1; ScalarPattern::Str(s) }
                    _ => return Err(c.err("E_SYNTAX",
                        "field value must be a literal or '_'; use 'is ~' for type fields")),
                };
                fields.push(FieldPattern { name, optional, value });
                if !c.eat(Tok::Comma) { break; }
                if c.tok() == &Tok::RBrace {
                    return Err(c.err("E_SYNTAX", "trailing value pattern comma"));
                }
            }
            c.expect(Tok::RBrace)?;
            PatternKind::ValueFields(fields)
        } else {
            let mut name = c.ident()?;
            while c.eat(Tok::Dot) || c.eat(Tok::ColonColon) {
                name.push('.');
                name.push_str(&c.ident()?);
            }
            PatternKind::ValueConstant(name)
        }
    };
    let binder = if c.eat_word("as") {
        if !(c.eat_word("val") || c.eat_word("let")) {
            return Err(c.err("E_SYNTAX", "expected val/let binder"));
        }
        Some(c.ident()?)
    } else { None };
    let guarded = c.eat_word("when");
    if guarded {
        if c.tok() == &Tok::Eof { return Err(c.err("E_SYNTAX", "missing guard")); }
        // Syntax-only: full guard expression, effects and ownership are
        // checked by the production matcher once it supports this syntax.
        c.i = c.ts.len()-1;
    }
    c.finish()?;
    Ok(Pattern { kind, binder, guarded })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn prefix_tilde_binding_ignores_whitespace_and_comments() {
        // Type formals, including both type-first and name-first forms.
        let a = parse_formals("(~{x: int, y: bool} point, other: ~Point)").unwrap();
        let b = parse_formals("(~ {x: int, y: bool} point, other: ~ Point)").unwrap();
        let spaced = parse_formals("(~
          /* comment */ {x: int, y: bool} point, other: ~
          Point)").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, spaced);

        // Explicit 'is' is a type test. Bare '~' is a value pattern.
        for (compact, spaced, comment) in [
            ("on is ~{x: int} as val p",
             "on is ~ {x: int} as val p",
             "on is ~/*comment*/{x: int} as val p"),
            ("on ~{x: 10, y: _} as val p",
             "on ~ {x: 10, y: _} as val p",
             "on ~\n{x: 10, y: _} as val p"),
            ("on ~Point as val p",
             "on ~ Point as val p",
             "on ~/*comment*/Point as val p"),
        ] {
            let target = parse_pattern(compact).unwrap();
            assert_eq!(target, parse_pattern(spaced).unwrap(), "{spaced}");
            assert_eq!(target, parse_pattern(comment).unwrap(), "{comment}");
        }

        // In ordinary argument expressions, '~ (' remains a Map/Record
        // aggregate, and whitespace cannot turn it into a type cast.
        for (compact, spaced, kind) in [
            ("(payload: ~(field: 42))",
             "(payload: ~ (field: 42))", ArgKind::RecordAggregate),
            ("(payload: ~(key => 42))",
             "(payload: ~ (key => 42))", ArgKind::MapAggregate),
            ("(~point, other: ~value)",
             "(~ point, other: ~ value)", ArgKind::StructuralProjection),
        ] {
            let left = parse_arguments(compact).unwrap();
            let right = parse_arguments(spaced).unwrap();
            assert_eq!(left.len(), right.len());
            assert_eq!(left[0].kind, kind);
            for (l, r) in left.iter().zip(right.iter()) {
                assert_eq!(l.kind, r.kind);
                assert_eq!(l.label, r.label);
            }
        }

        // Both spellings of a shape in expression position stay invalid.
        for invalid in ["(~{x: 10})", "(~ {x: 10})",
                        "(~{})", "(~ {})", "(payload: ~{x: 10})",
                        "(payload: ~ {x: 10})"] {
            assert!(parse_arguments(invalid).is_err(), "{invalid}");
        }
        // An empty match value-pattern is not the wildcard.
        for invalid in ["on ~{}", "on ~ {}", "on ~ /*comment*/{}"] {
            assert!(parse_pattern(invalid).is_err(), "{invalid}");
        }
    }

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
        let t = parse_pattern("on is ~Point as val p").unwrap();
        let v = parse_pattern("on ~Point as val p").unwrap();
        assert!(matches!(t.kind, PatternKind::TypeTest(_)));
        assert_eq!(v.kind, PatternKind::ValueConstant("Point".into()));
        assert_eq!(parse_pattern("on ~Point as val p").unwrap(),
                   parse_pattern("on structural Point as val p").unwrap());
        assert_eq!(parse_pattern("on ~{x: 10, y: _} as val p").unwrap(),
                   parse_pattern("on structural {x: 10, y: _} as val p").unwrap());
        let fields = parse_pattern("on ~{x: 10, y: _} as let p when p.x > 0").unwrap();
        assert!(fields.guarded);
        assert_eq!(fields.kind, PatternKind::ValueFields(vec![
            FieldPattern { name: "x".into(), optional: false, value: ScalarPattern::Int(10) },
            FieldPattern { name: "y".into(), optional: false, value: ScalarPattern::Any },
        ]));
        assert!(matches!(parse_pattern("on is ~{x: 10}").unwrap().kind,
                         PatternKind::TypeTest(_)));
        for bad in ["on ~{x: int}", "on ~{}", "on ~{x: 1, x: 2}",
                    "on is ~Point as p"] {
            assert!(parse_pattern(bad).is_err(), "{bad}");
        }
        assert!(parse_pattern("on Point").is_err());
        assert!(parse_pattern("on ~").is_err());
    }
    #[test] fn optional_value_fields_preserve_presence_bit_and_value_pattern() {
        let target = parse_pattern("on ~{ x?: 10, y: _ } as val p").unwrap();
        assert_eq!(target.kind, PatternKind::ValueFields(vec![
            FieldPattern { name: "x".into(), optional: true, value: ScalarPattern::Int(10) },
            FieldPattern { name: "y".into(), optional: false, value: ScalarPattern::Any },
        ]));
        // Whitespace/comments cannot change operator or presence meaning.
        for alias in [
            "on ~ {x?: 10, y: _} as val p",
            "on ~{x? : 10, y: _} as val p",
            "on structural { x?: 10, y: _ } as val p",
            "on ~/*c*/{ x? /*c*/ : 10, y: _ } as val p",
        ] {
            assert_eq!(target, parse_pattern(alias).unwrap(), "{alias}");
        }
        assert_eq!(
            parse_pattern("on ~{x?: _, y: _}").unwrap().kind,
            PatternKind::ValueFields(vec![
                FieldPattern { name: "x".into(), optional: true, value: ScalarPattern::Any },
                FieldPattern { name: "y".into(), optional: false, value: ScalarPattern::Any },
            ])
        );
        assert_eq!(
            parse_pattern("on ~{x: 10, y?: _}").unwrap().kind,
            PatternKind::ValueFields(vec![
                FieldPattern { name: "x".into(), optional: false, value: ScalarPattern::Int(10) },
                FieldPattern { name: "y".into(), optional: true, value: ScalarPattern::Any },
            ])
        );
    }

    #[test] fn optional_field_syntax_rejects_duplicate_unknown_or_missing_value() {
        for bad in [
            "on ~{x?:}",
            "on ~{x?: int}",
            "on ~{x??: 10}",
            "on ~{x?: 10, x: 10}",
            "on ~{x: 10, x?: 10}",
            "on ~{x?: effectful()}",
            "on ~{?: 10}",
            "on is ~{x?: 10}",
        ] {
            assert!(parse_pattern(bad).is_err(), "{bad}");
        }
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
