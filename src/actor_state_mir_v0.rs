//! Source-bound, restricted actor-state borrow event export (V0).
//!
//! This is NOT production typed CFG/NLL, an authenticated whole-body effect
//! summary, a guest ABI, or a proof of semantic parity. Only one concrete
//! actor and one method containing one flat, side-effect-free critical
//! callback are admitted. A real source is parsed through the production
//! actor parser; direct scalar field types and ceiling rights are resolved.
//! Any call, alias, conditional, return, nested operation or unknown source
//! effect is rejected instead of emitted as a success receipt.

use crate::ast::Type;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{lex, Tok, Token};
use crate::target_state_effects::{
    validate_source_actor_state_projections, StateLayoutModel, StatePlace, WriteCeiling,
};
use crate::target_syntax::parse_target_declarations;

const MAX_EVENTS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldEventKind { Read, Write }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldEvent {
    pub kind: FieldEventKind,
    pub field: usize,
    pub pos: Pos,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedGuardMir {
    pub actor: String,
    pub method: String,
    pub source_sha256: String,
    pub field_count: usize,
    pub exclusive: bool,
    pub events: Vec<FieldEvent>,
}

fn bad(t: &[Token], at: usize, code: &'static str, msg: &'static str) -> Diagnostic {
    Diagnostic::new(t.get(at).map(|token| token.pos).unwrap_or_default(), code, msg)
}

fn is_word(t: &[Token], at: usize, name: &str) -> bool {
    matches!(t.get(at).map(|x| &x.tok), Some(Tok::Ident(s)) if s == name)
}

fn expect(t: &[Token], cursor: &mut usize, expected: &Tok) -> Result<(), Diagnostic> {
    if t.get(*cursor).is_some_and(|x| &x.tok == expected) {
        *cursor += 1;
        Ok(())
    } else {
        Err(bad(t, *cursor, "E_MIR_UNSUPPORTED_EFFECT",
            "non-flat or unverified guarded actor-state operation"))
    }
}

fn expect_word(t: &[Token], cursor: &mut usize, name: &str) -> Result<(), Diagnostic> {
    if is_word(t, *cursor, name) {
        *cursor += 1;
        Ok(())
    } else {
        Err(bad(t, *cursor, "E_MIR_UNSUPPORTED_EFFECT",
            "unverified actor-state operation or receiver binding"))
    }
}

/// Derive a restricted flat guard event sequence from *actual source bytes*.
/// No caller may provide or edit the resulting field IDs, effects or digest.
pub fn derive_bounded_guard_mir(source: &str) -> Result<BoundedGuardMir, Diagnostic> {
    let toks = lex(source)?;
    let headers = parse_target_declarations(&toks)?;
    let _ = validate_source_actor_state_projections(&headers)?;
    if headers.receivers.len() != 1 || headers.receiver_sites.len() != 1 ||
       headers.definitions.len() != 1 || !headers.classes.is_empty() ||
       !headers.class_definitions.is_empty() || !headers.imports.is_empty() ||
       headers.namespace.is_some() || !headers.constructors.is_empty() ||
       !headers.struct_forwards.is_empty() || !headers.class_forwards.is_empty() ||
       !headers.state_owners.is_empty() {
        return Err(bad(&toks, 0, "E_MIR_UNSUPPORTED_EFFECT",
            "restricted source MIR requires exactly one local actor and one receiver"));
    }
    let receiver = &headers.receivers[0];
    let site = &headers.receiver_sites[0];
    let actor = &headers.definitions[0];
    if receiver.owner != actor.name || receiver.arity != 0 ||
       receiver.return_type != "void" || receiver.binding == "_" ||
       actor.receive.is_some() || !actor.methods.is_empty() ||
       actor.fields.is_empty() {
        return Err(bad(&toks, 0, "E_MIR_UNSUPPORTED_EFFECT",
            "receiver body or actor implementation cannot be audited in this profile"));
    }
    let names: Vec<&str> = actor.fields.iter().map(|f| f.name.as_str()).collect();
    let layout = StateLayoutModel::new(&actor.name, &names, actor.pos)?;
    let ceiling = layout.receiver_ceiling(receiver)?;
    let start = (site.method_start..site.end).find(|i|
        matches!(toks.get(*i).map(|t| &t.tok), Some(Tok::LBrace))
    ).ok_or_else(|| bad(&toks, site.method_start, "E_MIR_UNSUPPORTED_EFFECT",
        "guard method must use an explicit brace body"))?;
    let mut i = start + 1;
    expect_word(&toks, &mut i, &receiver.binding)?;
    expect(&toks, &mut i, &Tok::Dot)?;
    expect_word(&toks, &mut i, "handle_state")?;
    expect(&toks, &mut i, &Tok::LParen)?;
    expect_word(&toks, &mut i, "critical")?;
    expect(&toks, &mut i, &Tok::Minus)?;
    let exclusive = if is_word(&toks, i, "write") { true } else if
        is_word(&toks, i, "read") { false } else {
        return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
            "unknown actor critical guard kind"));
    };
    i += 1;
    if exclusive && !receiver.mutable {
        return Err(bad(&toks, i, "E_MUTABILITY", "critical-write needs mutable receiver"));
    }
    expect(&toks, &mut i, &Tok::Pipe)?;
    let binder = match toks.get(i).map(|t| &t.tok) {
        Some(Tok::Ident(name)) if name != &receiver.binding && name != "_" && name != "self" => name.clone(),
        _ => return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
            "invalid guard binder")),
    };
    i += 1;
    expect(&toks, &mut i, &Tok::Pipe)?;
    expect(&toks, &mut i, &Tok::Arrow)?;
    expect(&toks, &mut i, &Tok::LBrace)?;
    let mut events = Vec::new();
    while !matches!(toks.get(i).map(|t| &t.tok), Some(Tok::RBrace)) {
        if events.len() >= MAX_EVENTS {
            return Err(bad(&toks, i, "E_SOURCE_LIMIT", "actor guard event budget exceeded"));
        }
        expect_word(&toks, &mut i, &binder)?;
        expect(&toks, &mut i, &Tok::Dot)?;
        let pos = toks.get(i).map(|t| t.pos).unwrap_or_default();
        let field_name = match toks.get(i).map(|t| &t.tok) {
            Some(Tok::Ident(name)) => name.clone(),
            _ => return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
                "only direct scalar fields can be exported")),
        };
        let StatePlace::Field(field) = layout.place(Some(&field_name), pos)? else {
            unreachable!("named field always resolves to concrete field");
        };
        if actor.fields[field].ty != Type::Int {
            return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
                "only int typed actor-state fields can enter the V0 model"));
        }
        i += 1;
        let kind = match toks.get(i).map(|t| &t.tok) {
            Some(Tok::Semi) => FieldEventKind::Read,
            Some(Tok::Assign | Tok::PlusAssign) => {
                let compound = matches!(toks[i].tok, Tok::PlusAssign);
                i += 1;
                if !matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Int(Some(n))) if *n <= i64::MAX as u64) {
                    return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
                        "only bounded literal scalar writes can enter the V0 model"));
                }
                i += 1;
                if !exclusive || !actor.fields[field].binding.rebindable() ||
                    !matches!((&ceiling, field),
                        (WriteCeiling::Whole, _) |
                        (WriteCeiling::Fields(_), _))
                {
                    return Err(bad(&toks, i, "E_MUTABILITY", "unauthorized guarded state write"));
                }
                if let WriteCeiling::Fields(allowed) = &ceiling {
                    if !allowed.contains(&field) {
                        return Err(bad(&toks, i, "E_MUTABILITY", "guard write exceeds field ceiling"));
                    }
                }
                if compound {
                    events.push(FieldEvent { kind: FieldEventKind::Read, field, pos });
                }
                FieldEventKind::Write
            }
            _ => return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
                "call, branch, nested place, alias, control-flow or unknown effect")),
        };
        expect(&toks, &mut i, &Tok::Semi)?;
        events.push(FieldEvent { kind, field, pos });
    }
    if events.is_empty() {
        return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
            "empty guard is not a verified event inventory"));
    }
    expect(&toks, &mut i, &Tok::RBrace)?;
    expect(&toks, &mut i, &Tok::RParen)?;
    // One expression statement must be all of the method body.
    expect(&toks, &mut i, &Tok::Semi)?;
    expect(&toks, &mut i, &Tok::RBrace)?;
    if i != site.end {
        return Err(bad(&toks, i, "E_MIR_UNSUPPORTED_EFFECT",
            "trailing unverified receiver effects"));
    }
    Ok(BoundedGuardMir {
        actor: actor.name.clone(),
        method: receiver.name.clone(),
        source_sha256: crate::sha256::hex_digest(source.as_bytes()),
        field_count: actor.fields.len(),
        exclusive,
        events,
    })
}

impl BoundedGuardMir {
    /// Bounded, ASCII, strictly versioned textual transport. Canonical checker
    /// separately validates every record before constructing its own MIR.
    pub fn to_wire(&self) -> String {
        let mut s = format!("ATTO_ACTOR_STATE_MIR_V0\t{}\nROOT\t1\t{}\nLOAN\t1\t{}\n",
            self.source_sha256, self.field_count,
            if self.exclusive { "EXCLUSIVE" } else { "SHARED" });
        for e in &self.events {
            s.push_str(&format!("{}\t{}\t{}\t{}\n",
                if e.kind == FieldEventKind::Read { "READ" } else { "WRITE" },
                e.field, e.pos.line, e.pos.col));
        }
        s.push_str("END\t1\nDONE\n");
        s
    }
}
