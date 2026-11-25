//! Narrow file-local, closed signed-i64 quantity guest profile.
//! No imports, runtime conversions, affine values or general quantity ABI.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::quantity_scale::{ConversionError, Scale};
use std::collections::{BTreeMap, BTreeSet};

#[path = "verified_quantity_bindings.rs"]
mod bindings;

type R<T> = Result<T, Diagnostic>;
type Powers = BTreeMap<String, i32>;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit { powers: Powers, scale: Scale }
impl Unit {
    fn scalar() -> Self { Self { powers: BTreeMap::new(), scale: Scale::ONE } }
    fn combine(&self, other: &Self, divide: bool, pos: Pos) -> R<Self> {
        let mut powers = self.powers.clone();
        for (name, exponent) in &other.powers {
            let next = powers.get(name).copied().unwrap_or(0) + if divide { -*exponent } else { *exponent };
            if !(-64..=64).contains(&next) { return Err(error(pos, "E_UNIT_LIMIT", "dimension exponent exceeds 64")); }
            if next == 0 { powers.remove(name); } else { powers.insert(name.clone(), next); }
        }
        if powers.len() > 32 { return Err(error(pos, "E_UNIT_LIMIT", "more than 32 dimensions")); }
        let scale = self.scale.mul(if divide { other.scale.inv() } else { other.scale })
            .ok_or_else(|| error(pos, "E_UNIT_LIMIT", "normalized scale exceeds 256 bits"))?;
        Ok(Self { powers, scale })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArithmeticFailure { Overflow, DivisionByZero }
pub(crate) type Outcome = Result<i64, ArithmeticFailure>;
pub(crate) enum Lowered { Closed(Outcome), Typed(crate::check::TProgram) }
#[derive(Clone)]
struct Value { unit: Unit, number: Outcome }
fn error(pos: Pos, code: &'static str, message: &str) -> Diagnostic { Diagnostic::new(pos, code, message) }
fn checked(value: Option<i64>) -> Outcome {
    value.ok_or(ArithmeticFailure::Overflow)
}
fn attach(source: &Unit, target: Unit, pos: Pos) -> R<Unit> {
    if *source != Unit::scalar() {
        return Err(error(pos, "E_UNIT_ATTACHMENT", "unit attachment requires a dimensionless scale-one number; use to_unit for conversion"));
    }
    Ok(target)
}
fn convert(value: Value, unit: Unit, pos: Pos) -> R<Value> {
    if value.unit.powers != unit.powers { return Err(error(pos, "E_UNIT_DIMENSION", "conversion dimensions differ")); }
    let number = match value.number {
        Ok(n) => Ok(value.unit.scale.convert(n, unit.scale).map_err(|failure| match failure {
        ConversionError::Inexact => error(pos, "E_UNIT_INEXACT", "conversion is not integral"),
        ConversionError::Range => error(pos, "E_UNIT_RANGE", "conversion exceeds signed i64"),
        })?),
        Err(failure) => Err(failure),
    };
    Ok(Value { unit, number })
}

pub(crate) fn has_declarations(tokens: &[Token]) -> bool {
    tokens.windows(2).any(|w| matches!((&w[0].tok,&w[1].tok),
        (Tok::Ident(a),Tok::Ident(b)) if a == "define" && (b == "dimension" || b == "unit")))
}
pub(crate) fn lower(source: &str, tokens: &[Token]) -> R<Option<Lowered>> {
    if !has_declarations(tokens) { return Ok(None); }
    let mut p = Parser { tokens, at: 0, lines: source.lines().map(|l| l.chars().collect()).collect(),
        dimensions: BTreeSet::new(), bases: BTreeSet::new(), units: BTreeMap::new(), budget: 256 };
    let mut declarations = 0;
    while p.word("define") {
        declarations += 1;
        if declarations > 128 { return Err(p.err("E_UNIT_LIMIT", "too many quantity declarations")); }
        p.declaration()?;
    }
    if matches!(&p.token().tok, Tok::Ident(name) if name == "type") {
        return bindings::lower(&mut p).map(|program| Some(Lowered::Typed(program)));
    }
    p.expect_word("pub")?; p.expect_word("fnc")?; p.expect_word("main")?;
    p.expect(Tok::LParen)?; p.expect(Tok::RParen)?; p.expect(Tok::Arrow)?;
    p.expect_word("int")?; p.expect(Tok::LBrace)?; p.expect_word("return")?;
    let result = p.expr(0, 0)?;
    p.expect(Tok::Semi)?; p.expect(Tok::RBrace)?; p.expect(Tok::Eof)?;
    if result.unit != Unit::scalar() { return Err(p.err("E_UNIT_ERASURE", "int return requires scalar scale one")); }
    // Complete grammar and all arithmetic were checked before erasure. Ordinary
    // compiler emits the Wasm guest; there is no runtime solver or source eval.
    Ok(Some(Lowered::Closed(result.number)))
}

struct Parser<'a> {
    tokens: &'a [Token], at: usize, lines: Vec<Vec<char>>,
    dimensions: BTreeSet<String>, bases: BTreeSet<String>, units: BTreeMap<String,Unit>, budget: usize,
}
impl Parser<'_> {
    fn token(&self) -> &Token { &self.tokens[self.at.min(self.tokens.len()-1)] }
    fn err(&self, code: &'static str, message: &str) -> Diagnostic { error(self.token().pos, code, message) }
    fn take(&mut self, tok: Tok) -> bool {
        if self.token().tok == tok { self.at += 1; true } else { false }
    }
    fn expect(&mut self, tok: Tok) -> R<()> {
        if self.take(tok) { Ok(()) } else { Err(self.err("E_SYNTAX", "unexpected quantity token")) }
    }
    fn word(&mut self, name: &str) -> bool {
        if matches!(&self.token().tok, Tok::Ident(w) if w == name) { self.at += 1; true } else { false }
    }
    fn expect_word(&mut self, name: &str) -> R<()> {
        if self.word(name) { Ok(()) } else { Err(self.err("E_SYNTAX", "unexpected quantity keyword")) }
    }
    fn name(&mut self) -> R<String> {
        match self.token().tok.clone() {
            Tok::Ident(name) => { self.at += 1; Ok(name) }
            _ => Err(self.err("E_UNIT_NAME", "expected declaration name")),
        }
    }
    fn lookup(&self, name: &str) -> R<Unit> {
        self.units.get(name).cloned().ok_or_else(|| self.err("E_UNIT_NAME", "unknown or forward unit name"))
    }
    // The existing ColonColon token stays unchanged. Only consume it in
    // this quantity expression context; ordinary callable headers are untouched.
    fn checked_scalar_cast(&self, source: &Unit, name: &str, pos: Pos) -> R<Unit> {
        if !matches!(name, "int" | "i64") {
            return Err(error(pos, "E_UNSUPPORTED_CAST", "quantity profile supports only signed-i64 identity casts"));
        }
        if *source != Unit::scalar() {
            return Err(error(pos, "E_UNIT_ERASURE", "scalar cast cannot erase units"));
        }
        Ok(Unit::scalar())
    }
    fn cast_or_legacy_unit(&self, source: &Unit, name: &str, pos: Pos) -> R<Unit> {
        if matches!(name, "int" | "i64") {
            self.checked_scalar_cast(source, name, pos)
        } else {
            attach(source, self.lookup(name)?, pos)
        }
    }
    fn postfix_unit(&mut self, source: &Unit, pos: Pos) -> R<Option<Unit>> {
        let unit_only = self.token().tok == Tok::Dot
            && self.tokens.get(self.at + 1).is_some_and(|t| t.tok == Tok::Lt);
        if unit_only { self.at += 1; }
        else if !self.take(Tok::ColonColon) { return Ok(None); }
        if self.budget == 0 { return Err(self.err("E_UNIT_LIMIT", "quantity postfix budget")); }
        self.budget -= 1;
        self.expect(Tok::Lt)?;
        let name = self.name()?;
        self.expect(Tok::Gt)?;
        if unit_only { attach(source, self.lookup(&name)?, pos).map(Some) }
        else { self.cast_or_legacy_unit(source, &name, pos).map(Some) }
    }
    fn attach_postfix(&mut self, mut value: Value, pos: Pos) -> R<Value> {
        while let Some(unit) = self.postfix_unit(&value.unit, pos)? {
            value.unit = unit;
        }
        Ok(value)
    }
    fn factor(&mut self) -> R<Scale> {
        match self.token().tok {
            Tok::Int(Some(n)) if n > 0 && self.decimal_end(self.token().pos).is_some() => {
                self.at += 1;
                Scale::positive(n).ok_or_else(|| self.err("E_UNIT_SCALE", "expected positive decimal scale"))
            }
            _ => Err(self.err("E_UNIT_SCALE", "expected positive signed-i64 scale")),
        }
    }
    fn declaration(&mut self) -> R<()> {
        let dimension = self.word("dimension");
        if !dimension { self.expect_word("unit")?; }
        let name = self.name()?;
        if crate::parser::is_reserved(&name) || matches!(name.as_str(), "int" | "i32" | "bool" | "void" | "float" | "string") {
            return Err(self.err("E_RESERVED", "quantity declaration cannot shadow a reserved type or keyword"));
        }
        if self.dimensions.contains(&name) || self.units.contains_key(&name) {
            return Err(self.err("E_UNIT_DUPLICATE", "duplicate quantity name"));
        }
        if dimension {
            self.expect(Tok::Semi)?;
            self.dimensions.insert(name);
            return Ok(());
        }
        self.expect(Tok::Colon)?;
        let dim = self.name()?;
        if !self.dimensions.contains(&dim) { return Err(self.err("E_UNIT_NAME", "unknown dimension")); }
        let powers = BTreeMap::from([(dim.clone(),1)]);
        let unit = if self.take(Tok::Assign) {
            let (base, factor) = if matches!(self.token().tok, Tok::Int(_) | Tok::Minus) {
                let f = self.factor()?;
                self.expect(Tok::Star)?;
                (self.name()?, f)
            } else {
                let base = self.name()?;
                let factor = if self.take(Tok::Star) { self.factor()? }
                    else if self.take(Tok::Slash) { self.factor()?.inv() } else { Scale::ONE };
                (base, factor)
            };
            let base = self.lookup(&base)?;
            if base.powers != powers { return Err(self.err("E_UNIT_DIMENSION", "derived unit has different dimension")); }
            let scale = base.scale.mul(factor).ok_or_else(|| self.err("E_UNIT_LIMIT", "normalized scale exceeds 256 bits"))?;
            Unit { powers, scale }
        } else {
            if !self.bases.insert(dim) { return Err(self.err("E_UNIT_DUPLICATE", "duplicate base unit")); }
            Unit { powers, scale: Scale::ONE }
        };
        self.expect(Tok::Semi)?;
        self.units.insert(name, unit);
        Ok(())
    }
    fn decimal_end(&self, pos: Pos) -> Option<u32> {
        let line = self.lines.get(pos.line.checked_sub(1)? as usize)?;
        let start = pos.col.checked_sub(1)? as usize;
        let digits = line.get(start..)?.iter().take_while(|c| c.is_ascii_digit() || **c == '_').count();
        if digits == 0 || line.get(start + digits).is_some_and(|c| c.is_alphabetic()) {
            None
        } else {
            Some(pos.col + digits as u32)
        }
    }
    fn atom(&mut self, depth: usize) -> R<Value> {
        if depth > 64 || self.budget == 0 { return Err(self.err("E_UNIT_LIMIT", "quantity expression budget")); }
        self.budget -= 1;
        let pos = self.token().pos;
        if self.take(Tok::Lt) {
            let name = self.name()?;
            self.expect(Tok::Gt)?;
            let mut v = self.atom(depth + 1)?;
            v.unit = self.checked_scalar_cast(&v.unit, &name, pos)?;
            return Ok(v);
        }
        if self.take(Tok::Minus) {
            if self.token().tok == Tok::Int(None) {
                let mut v = self.literal(true)?;
                if self.token().tok == Tok::Dot && !self.tokens.get(self.at + 1).is_some_and(|t| t.tok == Tok::Lt) { return Err(self.err("E_UNSUPPORTED", "signed minimum conversion preview not supported")); }
                v.number = Ok(i64::MIN);
                return self.attach_postfix(v, pos);
            }
            let mut v = self.atom(depth + 1)?;
            v.number = v.number.and_then(|n| checked(n.checked_neg()));
            return Ok(v);
        }
        let parenthesized = self.take(Tok::LParen);
        let mut v = if parenthesized {
            let inner = self.expr(0,depth+1)?;
            self.expect(Tok::RParen)?;
            inner
        } else {
            self.literal(false)?
        };
        v = self.attach_postfix(v, pos)?;
        if self.take(Tok::Dot) {
            if parenthesized || v.unit == Unit::scalar() { return Err(self.err("E_UNSUPPORTED", "conversion is restricted to quantity literals")); }
            self.expect_word("to_unit")?; self.expect(Tok::Lt)?;
            let name = self.name()?; self.expect(Tok::Gt)?;
            self.expect(Tok::LParen)?; self.expect(Tok::RParen)?;
            v = convert(v,self.lookup(&name)?,pos)?;
        }
        self.attach_postfix(v, pos)
    }
    fn literal(&mut self, min: bool) -> R<Value> {
        let pos = self.token().pos;
        let number = match self.token().tok {
            Tok::Int(Some(n)) if !min => n as i64,
            Tok::Int(None) if min => i64::MIN,
            _ => return Err(self.err("E_SYNTAX", "expected signed decimal integer")),
        };
        let end = self.decimal_end(pos);
        if end.is_none() { return Err(self.err("E_SYNTAX", "expected decimal integer")); }
        self.at += 1;
        let unit = if self.token().tok == Tok::Lt {
            if self.token().pos.line != pos.line || end != Some(self.token().pos.col) {
                return Err(self.err("E_SYNTAX", "quantity suffix must immediately follow decimal integer"));
            }
            self.at += 1;
            let name = self.name()?;
            self.expect(Tok::Gt)?;
            self.lookup(&name)?
        } else { Unit::scalar() };
        Ok(Value { number: Ok(number), unit })
    }
    fn expr(&mut self, precedence: u8, depth: usize) -> R<Value> {
        let mut left = self.atom(depth)?;
        loop {
            let pos = self.token().pos;
            let (op, level) = match self.token().tok {
                Tok::Plus => (Tok::Plus,1), Tok::Minus => (Tok::Minus,1),
                Tok::Star => (Tok::Star,2), Tok::Slash => (Tok::Slash,2), _ => break,
            };
            if level <= precedence { break; }
            self.at += 1;
            let right = self.expr(level,depth+1)?;
            let unit = match op {
                Tok::Plus | Tok::Minus => {
                    if left.unit.powers != right.unit.powers { return Err(error(pos,"E_UNIT_DIMENSION","mixed dimensions")); }
                    if left.unit != right.unit { return Err(error(pos,"E_UNIT_SCALE","mixed scales")); }
                    left.unit.clone()
                }
                Tok::Star => left.unit.combine(&right.unit,false,pos)?,
                Tok::Slash => {
                    if left.unit == Unit::scalar() && right.unit == Unit::scalar() {
                        return Err(error(pos,"E_UNIT_SCALAR_DIVISION","legacy scalar division is outside quantity preview"));
                    }
                    left.unit.combine(&right.unit,true,pos)?
                }
                _ => unreachable!(),
            };
            // Type-check both sides even when an earlier operation will trap.
            // The first failure in eager left-to-right evaluation survives.
            let number = left.number.and_then(|a| right.number.and_then(|b| match op {
                Tok::Plus => checked(a.checked_add(b)),
                Tok::Minus => checked(a.checked_sub(b)),
                Tok::Star => checked(a.checked_mul(b)),
                Tok::Slash if b == 0 => Err(ArithmeticFailure::DivisionByZero),
                Tok::Slash => checked(a.checked_div(b)),
                _ => unreachable!(),
            }));
            left = Value { unit, number };
        }
        // Like checked casts, 'as' has lower precedence than arithmetic.
        // Parenthesize attachment when it is an operand of another operation.
        if precedence == 0 && self.word("as") {
            let pos = self.token().pos;
            let name = self.name()?;
            left.unit = self.cast_or_legacy_unit(&left.unit, &name, pos)?;
        }
        Ok(left)
    }
}
