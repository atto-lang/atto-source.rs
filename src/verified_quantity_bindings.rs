//! Typed source slice for i64 quantity bindings and earlier private helpers.
//! Units are checked structurally before erasure into the scalar Wasm IR.
//! Inclusive signed-i64 refinement ranges; no contracts, i32 arithmetic, dynamic conversion, FFI or exported quantity ABI.
use super::{Parser, Unit, Value, error, convert, attach, R};
use crate::ast::{BinOp, Type};
use crate::check::{TE, TExpr, TFunc, TProgram, TStmt};
use crate::diag::Pos;
use crate::lexer::Tok;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Range { lo: i64, hi: i64, holes: u8 }
impl Range {
    const ZERO: u8 = 1;
    const NEG_ONE: u8 = 2;
    const FULL: Self = Self::new(i64::MIN, i64::MAX);
    const fn new(lo: i64, hi: i64) -> Self { Self {lo,hi,holes:0} }
    fn exact(v: i64) -> Self { Self::new(v,v) }
    fn bit(v: i64) -> u8 { match v { 0 => Self::ZERO, -1 => Self::NEG_ONE, _ => 0 } }
    fn has(self, v: i64) -> bool { self.lo <= v && v <= self.hi && self.holes & Self::bit(v) == 0 }
    fn contains(self, other: Self) -> bool {
        self.lo <= other.lo && other.hi <= self.hi &&
        (self.holes & Self::ZERO == 0 || !other.has(0)) &&
        (self.holes & Self::NEG_ONE == 0 || !other.has(-1))
    }
    fn normalized(mut self) -> Option<Self> {
        if self.lo > self.hi { return None; }
        while self.holes & Self::bit(self.lo) != 0 {
            self.lo = self.lo.checked_add(1)?;
        }
        while self.holes & Self::bit(self.hi) != 0 {
            self.hi = self.hi.checked_sub(1)?;
        }
        if self.lo > self.hi { return None; }
        // Canonicalize redundant exclusions outside the interval.
        if self.lo > 0 || self.hi < 0 { self.holes &= !Self::ZERO; }
        if self.lo > -1 || self.hi < -1 { self.holes &= !Self::NEG_ONE; }
        Some(self)
    }
    fn intersect(self, other: Self) -> Option<Self> {
        Self {lo:self.lo.max(other.lo),hi:self.hi.min(other.hi),holes:self.holes|other.holes}.normalized()
    }
    fn exclude(self, n: i64) -> Option<Self> {
        if !self.has(n) { return Some(self); }
        if n == self.lo { return self.lo.checked_add(1).and_then(|lo| Self {lo,..self}.normalized()); }
        if n == self.hi { return self.hi.checked_sub(1).and_then(|hi| Self {hi,..self}.normalized()); }
        Self {holes:self.holes|Self::bit(n),..self}.normalized()
    }
    fn join(self, other: Self) -> Self {
        let mut r=Self::new(self.lo.min(other.lo),self.hi.max(other.hi));
        for n in [0,-1] {
            if !self.has(n) && !other.has(n) { r.holes |= Self::bit(n); }
        }
        r.normalized().expect("union of nonempty ranges")
    }
    fn normal_bounds(lo: i128, hi: i128) -> Self {
        let lo = lo.max(i64::MIN as i128);
        let hi = hi.min(i64::MAX as i128);
        if lo > hi { Self::FULL } else { Self::new(lo as i64, hi as i64) }
    }
    // Unlike normal-result bounds, this proves every operand pair succeeds.
    // i128 exactly represents products of two signed i64 values.
    fn proves_binary(self, other: Self, op: BinOp) -> bool {
        let (a,b,c,d) = (self.lo as i128, self.hi as i128, other.lo as i128, other.hi as i128);
        let fits = |lo: i128, hi: i128| lo >= i64::MIN as i128 && hi <= i64::MAX as i128;
        match op {
            BinOp::Add => fits(a+c,b+d),
            BinOp::Sub => fits(a-d,b-c),
            BinOp::Mul => {
                let v = [a*c,a*d,b*c,b*d];
                fits(*v.iter().min().unwrap(),*v.iter().max().unwrap())
            }
            BinOp::Div => !other.has(0) && !(self.has(i64::MIN) && other.has(-1)),
            _ => false,
        }
    }
    fn binary(self, other: Self, op: BinOp) -> Self {
        let (a,b,c,d) = (self.lo as i128, self.hi as i128, other.lo as i128, other.hi as i128);
        match op {
            BinOp::Add => Self::normal_bounds(a+c,b+d),
            BinOp::Sub => Self::normal_bounds(a-d,b-c),
            BinOp::Mul => {
                let v = [a*c,a*d,b*c,b*d];
                Self::normal_bounds(*v.iter().min().unwrap(),*v.iter().max().unwrap())
            }
            BinOp::Div if c > 0 || d < 0 => {
                let v = [a/c,a/d,b/c,b/d];
                Self::normal_bounds(*v.iter().min().unwrap(),*v.iter().max().unwrap())
            }
            _ => Self::FULL,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Refined { unit: Unit, range: Range }
impl Refined {
    fn scalar() -> Self { Self { unit: Unit::scalar(), range: Range::FULL } }
}
struct Checked {
    unit: Unit,
    range: Range,
    ir: TE,
}
#[derive(Clone)]
struct Signature {
    index: u32,
    params: Vec<Refined>,
    result: Refined,
}
type Locals = BTreeMap<String, (u32, Refined)>;
type Aliases = BTreeMap<String, Refined>;
type Functions = BTreeMap<String, Signature>;

fn int(e: TExpr) -> TE { TE { e, ty: Type::Int } }
fn compatible(want: &Unit, got: &Unit, pos: Pos) -> R<()> {
    if want.powers != got.powers {
        return Err(error(pos, "E_UNIT_DIMENSION", "quantity boundary has incompatible dimension"));
    }
    if want.scale != got.scale {
        return Err(error(pos, "E_UNIT_SCALE", "quantity boundary requires explicit scale conversion"));
    }
    Ok(())
}
fn identifier(p: &mut Parser<'_>) -> R<String> {
    let pos = p.token().pos;
    let n = p.name()?;
    if crate::parser::is_reserved(&n) || matches!(n.as_str(), "int" | "i64" | "i32" | "bool" | "void" | "float" | "string") {
        return Err(error(pos, "E_RESERVED", "reserved quantity declaration or binding name"));
    }
    Ok(n)
}
fn annotation(p: &mut Parser<'_>, aliases: &Aliases) -> R<Refined> {
    let pos = p.token().pos;
    let n = p.name()?;
    if n == "int" || n == "i64" { return Ok(Refined::scalar()); }
    aliases.get(&n).cloned().ok_or_else(|| error(pos, "E_UNIT_NAME", "unknown quantity type alias"))
}
fn boundary(want: &Refined, got: &Checked, pos: Pos, code: &'static str) -> R<()> {
    compatible(&want.unit, &got.unit, pos)?;
    if !want.range.contains(got.range) {
        return Err(error(pos, code, "refinement is not proved on the normal result"));
    }
    Ok(())
}
fn signed_bound(p: &mut Parser<'_>) -> R<i64> {
    let neg = p.take(Tok::Minus);
    let n = match p.token().tok {
        Tok::Int(Some(v)) => if neg { -(v as i64) } else { v as i64 },
        Tok::Int(None) if neg => i64::MIN,
        _ => return Err(p.err("E_REFINEMENT_BOUND", "expected signed decimal i64 bound")),
    };
    if p.decimal_end(p.token().pos).is_none() {
        return Err(p.err("E_REFINEMENT_BOUND", "expected decimal bound"));
    }
    p.at += 1;
    Ok(n)
}
fn read_predicate(p: &mut Parser<'_>) -> R<Range> {
    p.expect_word("self")?;
    let op = p.token().tok.clone();
    if !matches!(op, Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge | Tok::EqEq)
        && !matches!(&op, Tok::Ident(n) if n == "eq" || n == "gte" || n == "lte") {
        return Err(p.err("E_REFINEMENT_BOUND", "expected inclusive interval predicate"));
    }
    p.at += 1;
    let n = signed_bound(p)?;
    let r = match op {
        Tok::Lt => n.checked_sub(1).map(|hi| Range::new(i64::MIN,hi)),
        Tok::Gt => n.checked_add(1).map(|lo| Range::new(lo,i64::MAX)),
        Tok::Le => Some(Range::new(i64::MIN,n)),
        Tok::Ge => Some(Range::new(n,i64::MAX)),
        Tok::Ident(nm) if nm == "lte" => Some(Range::new(i64::MIN,n)),
        Tok::Ident(nm) if nm == "gte" => Some(Range::new(n,i64::MAX)),
        _ => Some(Range::exact(n)),
    };
    r.ok_or_else(|| p.err("E_REFINEMENT_EMPTY", "empty refinement"))
}
fn cast(p: &Parser<'_>, aliases: &Aliases, mut value: Checked, target: &str, pos: Pos, legacy: bool) -> R<Checked> {
    if let Some(want) = aliases.get(target) {
        compatible(&want.unit, &value.unit, pos)?;
        if p.strict && !want.range.contains(value.range) {
            return Err(error(pos, "E_VERIFY_UNPROVEN", "refinement cast may fail"));
        }
        value.range = value.range.intersect(want.range).unwrap_or(want.range);
        value.ir = int(TExpr::RefinementCheck(Box::new(value.ir),want.range.lo,want.range.hi));
    } else {
        value.unit = if legacy { p.cast_or_legacy_unit(&value.unit,target,pos)? }
                     else { p.checked_scalar_cast(&value.unit,target,pos)? };
    }
    Ok(value)
}

pub(super) fn lower(p: &mut Parser<'_>) -> R<TProgram> {
    let mut aliases = Aliases::new();
    while p.word("type") {
        if aliases.len() >= 128 { return Err(p.err("E_UNIT_LIMIT", "too many quantity aliases")); }
        let name = identifier(p)?;
        if aliases.contains_key(&name) || p.units.contains_key(&name) || p.dimensions.contains(&name) {
            return Err(p.err("E_UNIT_DUPLICATE", "duplicate quantity alias"));
        }
        p.expect(Tok::Assign)?;
        p.expect_word("int")?;
        p.expect_word("where")?;
        let mut unit = None;
        let mut range = Range::FULL;
        loop {
            if p.word("units") {
                if unit.is_some() { return Err(p.err("E_UNIT_DUPLICATE", "duplicate units clause")); }
                p.expect_word("in")?;
                let n = p.name()?;
                unit = Some(p.lookup(&n)?);
            } else {
                range = range.intersect(read_predicate(p)?)
                    .ok_or_else(|| p.err("E_REFINEMENT_EMPTY", "contradictory refinement"))?;
            }
            if !p.word("and") { break; }
        }
        p.expect(Tok::Semi)?;
        aliases.insert(name, Refined { unit: unit.unwrap_or_else(Unit::scalar), range });
    }

    let mut funcs = Vec::new();
    let mut signatures = Functions::new();
    loop {
        if funcs.len() >= 32 { return Err(p.err("E_UNIT_LIMIT", "too many quantity functions")); }
        let public = p.word("pub");
        p.expect_word("fnc")?;
        let name = identifier(p)?;
        if (name == "main") != public || signatures.contains_key(&name) ||
            aliases.contains_key(&name) || p.units.contains_key(&name) || p.dimensions.contains(&name) {
            return Err(p.err("E_UNIT_NAME", "duplicate name or non-main public quantity function"));
        }
        p.expect(Tok::LParen)?;
        let mut params = Vec::new();
        let mut locals = Locals::new();
        if !p.take(Tok::RParen) {
            loop {
                if params.len() >= 16 { return Err(p.err("E_UNIT_LIMIT", "too many parameters")); }
                let unit = annotation(p, &aliases)?;
                let param = identifier(p)?;
                if locals.insert(param, (params.len() as u32, unit.clone())).is_some() {
                    return Err(p.err("E_UNIT_DUPLICATE", "duplicate parameter"));
                }
                params.push(unit);
                if p.take(Tok::RParen) { break; }
                p.expect(Tok::Comma)?;
            }
        }
        p.expect(Tok::Arrow)?;
        let result = annotation(p, &aliases)?;
        if public && (!params.is_empty() || result != Refined::scalar()) {
            return Err(p.err("E_UNIT_ERASURE", "main must be zero-argument plain int"));
        }
        p.expect(Tok::LBrace)?;
        let mut body = Vec::new();
        let mut slots = Vec::new();
        if p.strict {
            let mut mutable = Aliases::new();
            let (stmts, returns) = strict_block(p, &aliases, &signatures, &result,
                &mut locals, &mut mutable, &mut slots, params.len(), 0)?;
            if !returns { return Err(p.err("E_VERIFY_RETURN", "not every path returns")); }
            body = stmts;
        } else {
        while p.word("val") {
            if slots.len() >= 64 { return Err(p.err("E_UNIT_LIMIT", "too many immutable bindings")); }
            // Preserve inferred initializer type and both explicit annotation orders.
            let first = p.token().pos;
            let (binding, wanted) = if p.tokens.get(p.at + 1).is_some_and(|t| t.tok == Tok::Colon) {
                let binding = identifier(p)?;
                p.expect(Tok::Colon)?;
                (binding, Some(annotation(p, &aliases)?))
            } else if p.tokens.get(p.at + 1).is_some_and(|t| t.tok == Tok::Assign) {
                (identifier(p)?, None)
            } else {
                let wanted = annotation(p, &aliases)?;
                (identifier(p)?, Some(wanted))
            };
            if locals.contains_key(&binding) { return Err(p.err("E_UNIT_DUPLICATE", "duplicate immutable binding")); }
            p.expect(Tok::Assign)?;
            let value = expr(p, &aliases, &locals, &signatures, 0, 0)?;
            if let Some(want) = &wanted {
                boundary(want, &value, first, "E_REFINEMENT_BINDING")?;
            }
            p.expect(Tok::Semi)?;
            let slot = (params.len() + slots.len()) as u32;
            slots.push(Type::Int);
            locals.insert(binding, (slot, Refined { unit: value.unit, range: value.range }));
            body.push(TStmt::Set(slot, value.ir));
        }
        p.expect_word("return")?;
        let pos = p.token().pos;
        let value = expr(p, &aliases, &locals, &signatures, 0, 0)?;
        if public && value.unit != Unit::scalar() {
            return Err(error(pos, "E_UNIT_ERASURE", "plain main result requires scale-one scalar"));
        }
        boundary(&result, &value, pos, "E_REFINEMENT_RETURN")?;
        p.expect(Tok::Semi)?;
        p.expect(Tok::RBrace)?;
        body.push(TStmt::Return(Some(value.ir)));
        }
        let index = funcs.len() as u32;
        funcs.push(TFunc { name: name.clone(), params: vec![Type::Int;params.len()], ret: Type::Int, locals: slots, body });
        if public {
            p.expect(Tok::Eof)?;
            return Ok(TProgram { funcs, generators: vec![], main: index, imports: vec![], data: vec![],
                heap: false, globals: vec![], actors: vec![], strings: false, guest_eh: false,
                raise_tag: false, catch_depth: false });
        }
        signatures.insert(name, Signature { index, params, result });
    }
}

 // Closed strict CFG: no loops, aliases, suspension, shadowing or external calls.
 // Each local has a stable physical slot; mutable assignments replace its fact.
fn strict_block(p: &mut Parser<'_>, aliases: &Aliases, funcs: &Functions,
    result: &Refined, locals: &mut Locals, mutable: &mut Aliases,
    slots: &mut Vec<Type>, param_count: usize, depth: usize) -> R<(Vec<TStmt>, bool)> {
    if depth > 16 { return Err(p.err("E_UNIT_LIMIT", "strict block depth")); }
    let mut body = Vec::new();
    let mut returned = false;
    while !p.take(Tok::RBrace) {
        if returned { return Err(p.err("E_VERIFY_UNSUPPORTED", "statement after unconditional return")); }
        if p.budget == 0 { return Err(p.err("E_UNIT_LIMIT", "strict statement budget")); }
        p.budget -= 1;
        let pos = p.token().pos;
        if p.word("return") {
            let v = expr(p, aliases, locals, funcs, 0, 0)?;
            boundary(result, &v, pos, "E_REFINEMENT_RETURN")?;
            p.expect(Tok::Semi)?;
            body.push(TStmt::Return(Some(v.ir)));
            returned = true;
        } else if p.word("if") {
            let (cond, yes, no) = strict_condition(p, aliases, locals, funcs)?;
            p.expect(Tok::LBrace)?;
            let mut yes = yes;
            let mut no = no;
            let (a, ar) = strict_block(p, aliases, funcs, result, &mut yes,
                &mut mutable.clone(), slots, param_count, depth+1)?;
            let (b, br) = if p.word("else") {
                p.expect(Tok::LBrace)?;
                strict_block(p, aliases, funcs, result, &mut no,
                    &mut mutable.clone(), slots, param_count, depth+1)?
            } else { (Vec::new(), false) };
            // Only incoming bindings survive lexical exit. Returning arms
            // contribute no continuation; otherwise take the interval hull.
            for (name, (_, fact)) in locals.iter_mut() {
                let a = yes.get(name).expect("incoming local").1.range;
                let b = no.get(name).expect("incoming local").1.range;
                fact.range = if ar { b } else if br { a }
                    else { a.join(b) };
            }
            body.push(TStmt::If(vec![(cond, a)], Some(b)));
            returned = ar && br;
            if returned { body.push(TStmt::Unreachable); }
        } else {
            let is_val = p.word("val");
            let is_let = !is_val && p.word("let");
            if is_val || is_let {
                if slots.len() >= 64 { return Err(p.err("E_UNIT_LIMIT", "too many strict bindings")); }
                let (name, wanted) = if p.tokens.get(p.at+1).is_some_and(|t| t.tok == Tok::Colon) {
                    let name = identifier(p)?;
                    p.expect(Tok::Colon)?;
                    (name, Some(annotation(p, aliases)?))
                } else if p.tokens.get(p.at+1).is_some_and(|t| t.tok == Tok::Assign) {
                    (identifier(p)?, None)
                } else {
                    let want = annotation(p, aliases)?;
                    (identifier(p)?, Some(want))
                };
                if locals.contains_key(&name) { return Err(p.err("E_UNIT_DUPLICATE", "duplicate strict binding")); }
                if is_let && wanted.is_none() { return Err(p.err("E_VERIFY_UNSUPPORTED", "mutable binding needs explicit type")); }
                p.expect(Tok::Assign)?;
                let v = expr(p, aliases, locals, funcs, 0, 0)?;
                if let Some(want) = &wanted { boundary(want, &v, pos, "E_REFINEMENT_BINDING")?; }
                p.expect(Tok::Semi)?;
                let slot = (param_count + slots.len()) as u32;
                slots.push(Type::Int);
                if is_let {
                    if let Some(want) = wanted { mutable.insert(name.clone(), want); }
                }
                locals.insert(name, (slot, Refined { unit: v.unit, range: v.range }));
                body.push(TStmt::Set(slot, v.ir));
            } else {
                let name = identifier(p)?;
                let want = mutable.get(&name).ok_or_else(|| error(pos,"E_VERIFY_UNSUPPORTED",
                    "strict statement requires an existing mutable scalar binding"))?;
                let slot = locals.get(&name).ok_or_else(|| error(pos,"E_VERIFY_UNSUPPORTED",
                    "assignment outside local scope"))?.0;
                p.expect(Tok::Assign)?;
                let v = expr(p, aliases, locals, funcs, 0, 0)?;
                boundary(want, &v, pos, "E_REFINEMENT_BINDING")?;
                p.expect(Tok::Semi)?;
                // Replace, never intersect with the old fact.
                locals.insert(name, (slot, Refined { unit: v.unit, range: v.range }));
                body.push(TStmt::Set(slot, v.ir));
            }
        }
    }
    Ok((body, returned))
}

// No unreachable-path proof: contradictory restrictions retain incoming facts.
fn restrict(r: Range, op: BinOp, n: i64) -> Range {
    if op == BinOp::Ne { return r.exclude(n).unwrap_or(r); }
    let candidate = match op {
        BinOp::Lt => n.checked_sub(1).map(|hi| Range::new(i64::MIN,hi)),
        BinOp::Le => Some(Range::new(i64::MIN,n)),
        BinOp::Gt => n.checked_add(1).map(|lo| Range::new(lo,i64::MAX)),
        BinOp::Ge => Some(Range::new(n,i64::MAX)),
        BinOp::Eq => Some(Range::exact(n)),
        _ => Some(r),
    };
    candidate.and_then(|x|r.intersect(x)).unwrap_or(r)
}
fn join_facts(mut a: Locals, b: &Locals) -> Locals {
    for (name, (_, fact)) in &mut a {
        fact.range = fact.range.join(b.get(name).expect("same condition scope").1.range);
    }
    a
}
// The lexer emits two punctuation tokens; require contiguous spelling, not & &.
fn take_bool_pair(p: &mut Parser<'_>, tok: Tok) -> bool {
    let a = p.token();
    let Some(b) = p.tokens.get(p.at+1) else { return false; };
    if a.tok == tok && b.tok == tok &&
        a.pos.line == b.pos.line && a.pos.col.checked_add(1) == Some(b.pos.col) {
        p.at += 2;
        true
    } else { false }
}
fn bool_ir(e: TExpr) -> TE { TE { ty:Type::Bool, e } }
fn strict_condition(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals,
    funcs: &Functions) -> R<(TE,Locals,Locals)> {
    strict_or(p, aliases, locals, funcs, 0)
}
fn strict_or(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals,
    funcs: &Functions, depth: usize) -> R<(TE,Locals,Locals)> {
    let (mut ir, mut yes, mut no) = strict_and(p, aliases, locals, funcs, depth)?;
    while take_bool_pair(p, Tok::Pipe) {
        let (rhs, ry, rn) = strict_and(p, aliases, &no, funcs, depth)?;
        // OR evaluates RHS only on the left-false edge.
        ir = bool_ir(TExpr::Ternary(Box::new(ir),
            Box::new(bool_ir(TExpr::Bool(true))), Box::new(rhs)));
        yes = join_facts(yes, &ry);
        no = rn;
    }
    Ok((ir,yes,no))
}
fn strict_and(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals,
    funcs: &Functions, depth: usize) -> R<(TE,Locals,Locals)> {
    let (mut ir, mut yes, mut no) = strict_predicate(p, aliases, locals, funcs, depth)?;
    while take_bool_pair(p, Tok::Amp) {
        let (rhs, ry, rn) = strict_predicate(p, aliases, &yes, funcs, depth)?;
        // AND evaluates RHS only on the left-true edge.
        ir = bool_ir(TExpr::Ternary(Box::new(ir), Box::new(rhs),
            Box::new(bool_ir(TExpr::Bool(false)))));
        no = join_facts(no, &rn);
        yes = ry;
    }
    Ok((ir,yes,no))
}
fn strict_predicate(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals,
    funcs: &Functions, depth: usize) -> R<(TE,Locals,Locals)> {
    if depth > 16 || p.budget == 0 {
        return Err(p.err("E_UNIT_LIMIT","strict Boolean condition budget"));
    }
    p.budget -= 1;
    if p.take(Tok::Bang) {
        let (ir,yes,no) = strict_predicate(p,aliases,locals,funcs,depth+1)?;
        return Ok((bool_ir(TExpr::Not(Box::new(ir))),no,yes));
    }
    if p.take(Tok::LParen) {
        let result = strict_or(p,aliases,locals,funcs,depth+1)?;
        p.expect(Tok::RParen)?;
        return Ok(result);
    }
    let pos = p.token().pos;
    let left = expr(p,aliases,locals,funcs,0,0)?;
    let op = match &p.token().tok {
        Tok::Lt => BinOp::Lt, Tok::Le => BinOp::Le,
        Tok::Gt => BinOp::Gt, Tok::Ge => BinOp::Ge,
        Tok::EqEq => BinOp::Eq, Tok::NotEq => BinOp::Ne,
        Tok::Ident(s) if s == "eq" => BinOp::Eq,
        Tok::Ident(s) if s == "neq" => BinOp::Ne,
        _ => return Err(p.err("E_VERIFY_UNSUPPORTED", "strict condition requires a comparison")),
    };
    p.at += 1;
    let right = expr(p,aliases,locals,funcs,0,0)?;
    compatible(&left.unit,&right.unit,pos)?;
    let inverse = match op {
        BinOp::Lt => BinOp::Ge, BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le, BinOp::Ge => BinOp::Lt,
        BinOp::Eq => BinOp::Ne, BinOp::Ne => BinOp::Eq,
        _ => return Err(p.err("E_VERIFY_UNSUPPORTED","condition comparison")),
    };
    let mut yes = locals.clone(); let mut no = locals.clone();
    // Refine only a resolved local compared with a singleton value.
    // Calls are closed, scalar by value and already safety-checked.
    // Arithmetic or other RHSs cannot manufacture a refinement.
    if let TExpr::Local(slot) = &left.ir.e {
        if right.range.lo == right.range.hi {
            if let Some((name,(_,fact))) = locals.iter().find(|(_, (s,_))| s == slot) {
                yes.get_mut(name).expect("resolved local").1.range = restrict(fact.range,op,right.range.lo);
                no.get_mut(name).expect("resolved local").1.range = restrict(fact.range,inverse,right.range.lo);
            }
        }
    }
    let cond = bool_ir(TExpr::Bin(op,Box::new(left.ir),Box::new(right.ir)));
    Ok((cond,yes,no))
}

fn literal(p: &mut Parser<'_>, min: bool) -> R<Checked> {
    let pos = p.token().pos;
    let mut value: Value = p.literal(min)?;
    // Literal conversion retains its original restricted profile. Symbolic
    // casts are consumed after literal construction by the typed atom wrapper.
    // A literal unit attachment may precede exact conversion.
    while (p.token().tok == Tok::Dot || p.token().tok == Tok::ColonColon)
        && p.tokens.get(p.at+1).is_some_and(|t| t.tok == Tok::Lt)
        && p.tokens.get(p.at+2).is_some_and(|t| matches!(&t.tok, Tok::Ident(n) if p.units.contains_key(n))) {
        value.unit = p.postfix_unit(&value.unit, pos)?.expect("unit suffix");
    }
    if p.token().tok == Tok::Dot && !p.tokens.get(p.at+1).is_some_and(|t| t.tok == Tok::Lt) {
        p.at += 1;
        if min || value.unit == Unit::scalar() {
            return Err(p.err("E_UNSUPPORTED", "conversion requires ordinary quantity literal"));
        }
        p.expect_word("to_unit")?;
        p.expect(Tok::Lt)?;
        let target = p.name()?;
        p.expect(Tok::Gt)?;
        p.expect(Tok::LParen)?;
        p.expect(Tok::RParen)?;
        value = convert(value, p.lookup(&target)?, pos)?;
    }
    let number = value.number.map_err(|_| error(pos, "E_UNIT_RANGE", "literal cannot be represented"))?;
    Ok(Checked { unit: value.unit, range: Range::exact(number), ir: int(TExpr::I64(number)) })
}
fn atom(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals, funcs: &Functions, depth: usize) -> R<Checked> {
    let pos = p.token().pos;
    let mut value = atom_base(p, aliases, locals, funcs, depth)?;
    while p.token().tok == Tok::ColonColon ||
        (p.token().tok == Tok::Dot && p.tokens.get(p.at+1).is_some_and(|t| t.tok == Tok::Lt)) {
        if p.budget == 0 { return Err(p.err("E_UNIT_LIMIT", "typed postfix budget")); }
        p.budget -= 1;
        let unit_only = p.take(Tok::Dot);
        if !unit_only { p.expect(Tok::ColonColon)?; }
        p.expect(Tok::Lt)?;
        let target = p.name()?;
        p.expect(Tok::Gt)?;
        if unit_only { value.unit = attach(&value.unit,p.lookup(&target)?,pos)?; }
        else { value = cast(p,aliases,value,&target,pos,true)?; }
    }
    Ok(value)
}
fn atom_base(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals, funcs: &Functions, depth: usize) -> R<Checked> {
    if depth > 48 || p.budget == 0 { return Err(p.err("E_UNIT_LIMIT", "typed quantity expression budget")); }
    p.budget -= 1;
    if p.take(Tok::Lt) {
        let pos = p.token().pos;
        let target = p.name()?;
        p.expect(Tok::Gt)?;
        let value = atom(p, aliases, locals, funcs, depth+1)?;
        return cast(p,aliases,value,&target,pos,false);
    }
    let neg_pos = p.token().pos;
    if p.take(Tok::Minus) {
        if p.token().tok == Tok::Int(None) { return literal(p, true); }
        let value = atom(p, aliases, locals, funcs, depth+1)?;
        if p.strict && value.range.lo == i64::MIN {
            return Err(error(neg_pos, "E_VERIFY_UNPROVEN", "signed negation may overflow"));
        }
        let range = Range::normal_bounds(-(value.range.hi as i128),-(value.range.lo as i128));
        return Ok(Checked { unit: value.unit, range, ir: int(TExpr::Neg(Box::new(value.ir))) });
    }
    if p.take(Tok::LParen) {
        let value = expr(p, aliases, locals, funcs, 0, depth+1)?;
        p.expect(Tok::RParen)?;
        return Ok(value);
    }
    if matches!(p.token().tok, Tok::Int(_)) { return literal(p,false); }
    let pos = p.token().pos;
    let name = identifier(p)?;
    if p.take(Tok::LParen) {
        let sig = funcs.get(&name).ok_or_else(|| error(pos,"E_UNIT_NAME","unknown, recursive or forward quantity call"))?;
        let mut args = Vec::new();
        for (i, wanted) in sig.params.iter().enumerate() {
            if i != 0 { p.expect(Tok::Comma)?; }
            let arg = expr(p,aliases,locals,funcs,0,depth+1)?;
            boundary(wanted,&arg,pos,"E_REFINEMENT_PRECONDITION")?;
            args.push(arg.ir);
        }
        p.expect(Tok::RParen)?;
        return Ok(Checked { unit: sig.result.unit.clone(), range: sig.result.range, ir: int(TExpr::Call(sig.index,args)) });
    }
    let (slot,unit) = locals.get(&name).ok_or_else(|| error(pos,"E_UNIT_NAME","unknown immutable quantity binding"))?;
    Ok(Checked { unit:unit.unit.clone(), range: unit.range, ir:int(TExpr::Local(*slot)) })
}
fn expr(p: &mut Parser<'_>, aliases: &Aliases, locals: &Locals, funcs: &Functions, precedence: u8, depth: usize) -> R<Checked> {
    let mut left = atom(p,aliases,locals,funcs,depth)?;
    loop {
        let pos = p.token().pos;
        let (op,level) = match p.token().tok {
            Tok::Plus => (BinOp::Add,1), Tok::Minus => (BinOp::Sub,1),
            Tok::Star => (BinOp::Mul,2), Tok::Slash => (BinOp::Div,2), _ => break,
        };
        if level <= precedence { break; }
        p.at += 1;
        let right = expr(p,aliases,locals,funcs,level,depth+1)?;
        let unit = match op {
            BinOp::Add | BinOp::Sub => { compatible(&left.unit,&right.unit,pos)?; left.unit.clone() }
            BinOp::Mul => left.unit.combine(&right.unit,false,pos)?,
            BinOp::Div => {
                if !p.strict && left.unit == Unit::scalar() && right.unit == Unit::scalar() {
                    return Err(error(pos,"E_UNIT_SCALAR_DIVISION","ordinary int division remains outside quantity profile"));
                }
                left.unit.combine(&right.unit,true,pos)?
            }
            _ => unreachable!(),
        };
        if p.strict && !left.range.proves_binary(right.range,op) {
            return Err(error(pos,"E_VERIFY_UNPROVEN","signed arithmetic may overflow or divide by zero"));
        }
        let range = left.range.binary(right.range,op);
        left = Checked { unit, range, ir: int(TExpr::Bin(op,Box::new(left.ir),Box::new(right.ir))) };
    }
    if precedence == 0 && p.word("as") {
        let pos = p.token().pos;
        let name = p.name()?;
        left = cast(p,aliases,left,&name,pos,true)?;
    }
    Ok(left)
}

#[cfg(test)]
mod strict_tests {
    use super::*;
    #[test]
    fn branch_facts_cover_every_sampled_concrete_successor() {
        let mut samples = vec![i64::MIN, i64::MIN+1, i64::MIN+2];
        samples.extend(-8..=7);
        samples.extend([i64::MAX-2,i64::MAX-1,i64::MAX]);
        for (i,lo) in samples.iter().enumerate() {
            for hi in &samples[i..] {
                let r=Range::new(*lo,*hi);
                for n in &samples {
                    for (op,inv) in [(BinOp::Lt,BinOp::Ge),(BinOp::Le,BinOp::Gt),
                        (BinOp::Gt,BinOp::Le),(BinOp::Ge,BinOp::Lt),
                        (BinOp::Eq,BinOp::Ne),(BinOp::Ne,BinOp::Eq)] {
                        let yes=restrict(r,op,*n); let no=restrict(r,inv,*n);
                        assert!(r.contains(yes) && r.contains(no));
                        for x in samples.iter().filter(|x| r.lo <= **x && **x <= r.hi) {
                            let matches = match op {
                                BinOp::Lt => x<n, BinOp::Le => x<=n,
                                BinOp::Gt => x>n, BinOp::Ge => x>=n,
                                BinOp::Eq => x==n, BinOp::Ne => x!=n,
                                _ => unreachable!(),
                            };
                            assert!((if matches {yes} else {no}).contains(Range::exact(*x)));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn interval_admission_matches_exhaustive_small_domain() {
        for a in -8i64..=7 { for b in a..=7 {
            for c in -8i64..=7 { for d in c..=7 {
                let l=Range::new(a,b); let r=Range::new(c,d);
                for op in [BinOp::Add,BinOp::Sub,BinOp::Mul,BinOp::Div] {
                    let all=(a..=b).all(|x| (c..=d).all(|y| match op {
                        BinOp::Add => x.checked_add(y).is_some(),
                        BinOp::Sub => x.checked_sub(y).is_some(),
                        BinOp::Mul => x.checked_mul(y).is_some(),
                        BinOp::Div => x.checked_div(y).is_some(),
                        _ => false,
                    }));
                    assert_eq!(l.proves_binary(r,op),all);
                }
            }}
        }}
    }
    #[test]
    fn interval_admission_matches_checked_i64_near_extrema() {
        let mut ranges = Vec::new();
        for start in [i64::MIN,-3,0,i64::MAX-3] {
            for lo in 0..=3 { for hi in lo..=3 {
                ranges.push(Range::new(start+lo,start+hi));
            }}
        }
        for l in &ranges { for r in &ranges {
            for op in [BinOp::Add,BinOp::Sub,BinOp::Mul,BinOp::Div] {
                let all=(l.lo..=l.hi).all(|x| (r.lo..=r.hi).all(|y| match op {
                    BinOp::Add => x.checked_add(y).is_some(),
                    BinOp::Sub => x.checked_sub(y).is_some(),
                    BinOp::Mul => x.checked_mul(y).is_some(),
                    BinOp::Div => x.checked_div(y).is_some(),
                    _ => false,
                }));
                assert_eq!(l.proves_binary(*r,op),all);
            }
        }}
    }
    #[test]
    fn extrema_do_not_use_clipped_normal_result_as_proof() {
        let min=Range::exact(i64::MIN); let max=Range::exact(i64::MAX);
        let neg=Range::exact(-1); let one=Range::exact(1);
        let zero=Range::exact(0);
        assert!(!max.proves_binary(one,BinOp::Add));
        assert!(!min.proves_binary(one,BinOp::Sub));
        assert!(!min.proves_binary(neg,BinOp::Mul));
        assert!(!min.proves_binary(neg,BinOp::Div));
        assert!(!Range::FULL.proves_binary(zero,BinOp::Div));
        assert!(!Range::FULL.proves_binary(neg,BinOp::Div));
        assert!(Range::FULL.proves_binary(one,BinOp::Div));
        assert!(Range::FULL.proves_binary(zero,BinOp::Mul));
        assert!(!Range::new(i64::MAX-1,i64::MAX).proves_binary(one,BinOp::Add));
    }
}
