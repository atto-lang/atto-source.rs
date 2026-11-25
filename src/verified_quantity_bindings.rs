//! Typed source slice for immutable i64 quantity bindings and earlier private helpers.
//! Units are checked structurally before erasure into the scalar Wasm IR.
//! No refinement predicates, dynamic conversion, FFI, imports or exported quantity ABI.
use super::{Parser, Unit, Value, error, convert, R};
use crate::ast::{BinOp, Type};
use crate::check::{TE, TExpr, TFunc, TProgram, TStmt};
use crate::diag::Pos;
use crate::lexer::Tok;
use std::collections::BTreeMap;

struct Checked {
    unit: Unit,
    ir: TE,
}
#[derive(Clone)]
struct Signature {
    index: u32,
    params: Vec<Unit>,
    result: Unit,
}
type Locals = BTreeMap<String, (u32, Unit)>;
type Aliases = BTreeMap<String, Unit>;
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
    if crate::parser::is_reserved(&n) || matches!(n.as_str(), "int" | "i32" | "bool" | "void" | "float" | "string") {
        return Err(error(pos, "E_RESERVED", "reserved quantity declaration or binding name"));
    }
    Ok(n)
}
fn annotation(p: &mut Parser<'_>, aliases: &Aliases) -> R<Unit> {
    let pos = p.token().pos;
    let n = p.name()?;
    if n == "int" { return Ok(Unit::scalar()); }
    aliases.get(&n).cloned().ok_or_else(|| error(pos, "E_UNIT_NAME", "unknown quantity type alias"))
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
        p.expect_word("units")?;
        p.expect_word("in")?;
        let unit = p.name()?;
        let unit = p.lookup(&unit)?;
        p.expect(Tok::Semi)?;
        aliases.insert(name, unit);
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
        if public && (!params.is_empty() || result != Unit::scalar()) {
            return Err(p.err("E_UNIT_ERASURE", "main must be zero-argument plain int"));
        }
        p.expect(Tok::LBrace)?;
        let mut body = Vec::new();
        let mut slots = Vec::new();
        while p.word("val") {
            if slots.len() >= 64 { return Err(p.err("E_UNIT_LIMIT", "too many immutable bindings")); }
            // Both forms admitted by the scalar refinements proposal.
            let first = p.token().pos;
            let (binding, wanted) = if p.tokens.get(p.at + 1).is_some_and(|t| t.tok == Tok::Colon) {
                let binding = identifier(p)?;
                p.expect(Tok::Colon)?;
                (binding, annotation(p, &aliases)?)
            } else {
                let wanted = annotation(p, &aliases)?;
                (identifier(p)?, wanted)
            };
            if locals.contains_key(&binding) { return Err(p.err("E_UNIT_DUPLICATE", "duplicate immutable binding")); }
            p.expect(Tok::Assign)?;
            let value = expr(p, &locals, &signatures, 0, 0)?;
            compatible(&wanted, &value.unit, first)?;
            p.expect(Tok::Semi)?;
            let slot = (params.len() + slots.len()) as u32;
            slots.push(Type::Int);
            locals.insert(binding, (slot, wanted));
            body.push(TStmt::Set(slot, value.ir));
        }
        p.expect_word("return")?;
        let pos = p.token().pos;
        let value = expr(p, &locals, &signatures, 0, 0)?;
        if public && value.unit != Unit::scalar() {
            return Err(error(pos, "E_UNIT_ERASURE", "plain main result requires scale-one scalar"));
        }
        compatible(&result, &value.unit, pos)?;
        p.expect(Tok::Semi)?;
        p.expect(Tok::RBrace)?;
        body.push(TStmt::Return(Some(value.ir)));
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
fn literal(p: &mut Parser<'_>, min: bool) -> R<Checked> {
    let pos = p.token().pos;
    let mut value: Value = p.literal(min)?;
    value = p.attach_postfix(value, pos)?;
    if p.take(Tok::Dot) {
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
    Ok(Checked { unit: value.unit, ir: int(TExpr::I64(number)) })
}
fn atom(p: &mut Parser<'_>, locals: &Locals, funcs: &Functions, depth: usize) -> R<Checked> {
    let pos = p.token().pos;
    let mut value = atom_base(p, locals, funcs, depth)?;
    while let Some(unit) = p.postfix_unit(&value.unit, pos)? {
        value.unit = unit;
    }
    Ok(value)
}
fn atom_base(p: &mut Parser<'_>, locals: &Locals, funcs: &Functions, depth: usize) -> R<Checked> {
    if depth > 48 || p.budget == 0 { return Err(p.err("E_UNIT_LIMIT", "typed quantity expression budget")); }
    p.budget -= 1;
    if p.take(Tok::Lt) {
        let pos = p.token().pos;
        let target = p.name()?;
        p.expect(Tok::Gt)?;
        let mut value = atom(p, locals, funcs, depth+1)?;
        value.unit = p.checked_scalar_cast(&value.unit, &target, pos)?;
        return Ok(value);
    }
    if p.take(Tok::Minus) {
        if p.token().tok == Tok::Int(None) { return literal(p, true); }
        let value = atom(p, locals, funcs, depth+1)?;
        return Ok(Checked { unit: value.unit, ir: int(TExpr::Neg(Box::new(value.ir))) });
    }
    if p.take(Tok::LParen) {
        let value = expr(p, locals, funcs, 0, depth+1)?;
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
            let arg = expr(p,locals,funcs,0,depth+1)?;
            compatible(wanted,&arg.unit,pos)?;
            args.push(arg.ir);
        }
        p.expect(Tok::RParen)?;
        return Ok(Checked { unit: sig.result.clone(), ir: int(TExpr::Call(sig.index,args)) });
    }
    let (slot,unit) = locals.get(&name).ok_or_else(|| error(pos,"E_UNIT_NAME","unknown immutable quantity binding"))?;
    Ok(Checked { unit:unit.clone(), ir:int(TExpr::Local(*slot)) })
}
fn expr(p: &mut Parser<'_>, locals: &Locals, funcs: &Functions, precedence: u8, depth: usize) -> R<Checked> {
    let mut left = atom(p,locals,funcs,depth)?;
    loop {
        let pos = p.token().pos;
        let (op,level) = match p.token().tok {
            Tok::Plus => (BinOp::Add,1), Tok::Minus => (BinOp::Sub,1),
            Tok::Star => (BinOp::Mul,2), Tok::Slash => (BinOp::Div,2), _ => break,
        };
        if level <= precedence { break; }
        p.at += 1;
        let right = expr(p,locals,funcs,level,depth+1)?;
        let unit = match op {
            BinOp::Add | BinOp::Sub => { compatible(&left.unit,&right.unit,pos)?; left.unit.clone() }
            BinOp::Mul => left.unit.combine(&right.unit,false,pos)?,
            BinOp::Div => {
                if left.unit == Unit::scalar() && right.unit == Unit::scalar() {
                    return Err(error(pos,"E_UNIT_SCALAR_DIVISION","ordinary int division remains outside quantity profile"));
                }
                left.unit.combine(&right.unit,true,pos)?
            }
            _ => unreachable!(),
        };
        left = Checked { unit, ir: int(TExpr::Bin(op,Box::new(left.ir),Box::new(right.ir))) };
    }
    if precedence == 0 && p.word("as") {
        let pos = p.token().pos;
        let name = p.name()?;
        left.unit = p.cast_or_legacy_unit(&left.unit, &name, pos)?;
    }
    Ok(left)
}
