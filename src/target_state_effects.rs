//! Restricted actor-state place/loan oracle for target typed IR.
//!
//! This module does NOT typecheck method bodies, authenticate a state layout,
//! calculate CFG/NLL liveness, admit actor guests, or change critical lock
//! granularity. Callers must independently establish a sealed actor schema
//! and typed transitive effects before these checks can be used for admission.

use crate::diag::{Diagnostic, Pos};
use crate::target_syntax::{ReceiverHeader, ReceiverStateWrite};
use std::collections::BTreeSet;

type Check<T> = Result<T, Diagnostic>;

fn error(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

/// Canonical field identity within one resolved actor-state layout.
/// Field indices are local to this layout, never source-level names or offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatePlace {
    Whole,
    Field(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteCeiling {
    ReadOnly,
    Whole,
    Fields(BTreeSet<usize>),
}

/// A bounded host-side model of a previously resolved actor state schema.
/// Constructing it does not attest that the source compiler resolved a schema.
#[derive(Clone, Debug)]
pub struct StateLayoutModel {
    actor: String,
    fields: Vec<String>,
}

impl StateLayoutModel {
    pub fn new(actor: &str, fields: &[&str], pos: Pos) -> Check<Self> {
        if actor.is_empty() || fields.is_empty() || fields.len() > 32 {
            return Err(error(pos, "E_STATE_FIELD_UNKNOWN", "missing or oversized actor-state schema"));
        }
        let mut unique = BTreeSet::new();
        for field in fields {
            if field.is_empty() || !unique.insert(*field) {
                return Err(error(pos, "E_STATE_FIELD_UNKNOWN", "duplicate or empty state field identity"));
            }
        }
        Ok(Self {
            actor: actor.to_owned(),
            fields: fields.iter().map(|name| (*name).to_owned()).collect(),
        })
    }

    pub fn place(&self, field: Option<&str>, pos: Pos) -> Check<StatePlace> {
        match field {
            None => Ok(StatePlace::Whole),
            Some(name) => self.fields.iter().position(|candidate| candidate == name)
                .map(StatePlace::Field)
                .ok_or_else(|| error(pos, "E_STATE_FIELD_UNKNOWN",
                    format!("state field '{name}' is absent from resolved schema"))),
        }
    }

    fn validate_place(&self, place: StatePlace, pos: Pos) -> Check<()> {
        if matches!(place, StatePlace::Field(index) if index >= self.fields.len()) {
            return Err(error(pos, "E_STATE_FIELD_UNKNOWN", "unresolved canonical field identity"));
        }
        Ok(())
    }

    /// Resolve a declared write ceiling to canonical field IDs. This is not
    /// a grant to mutate actor state without a verified critical-write guard.
    pub fn receiver_ceiling(&self, header: &ReceiverHeader) -> Check<WriteCeiling> {
        if header.owner != self.actor {
            return Err(error(header.pos, "E_ACTOR", "receiver belongs to another actor"));
        }
        match &header.state_write {
            None => Ok(WriteCeiling::ReadOnly),
            Some(_) if !header.mutable =>
                Err(error(header.pos, "E_MUTABILITY", "state write ceiling requires mutable receiver")),
            Some(ReceiverStateWrite::Whole) => Ok(WriteCeiling::Whole),
            Some(ReceiverStateWrite::Fields(fields)) => {
                if fields.is_empty() || fields.len() > 32 {
                    return Err(error(header.pos, "E_STATE_FIELD_UNKNOWN", "invalid field projection size"));
                }
                let mut ids = BTreeSet::new();
                for field in fields {
                    let StatePlace::Field(id) = self.place(Some(field), header.pos)? else {
                        unreachable!("a named field always resolves to a field");
                    };
                    if !ids.insert(id) {
                        return Err(error(header.pos, "E_DUPLICATE", "duplicate state field in receiver"));
                    }
                }
                Ok(WriteCeiling::Fields(ids))
            }
        }
    }

    fn permits(&self, ceiling: &WriteCeiling, place: StatePlace) -> bool {
        match (ceiling, place) {
            (WriteCeiling::Whole, _) => true,
            (WriteCeiling::Fields(fields), StatePlace::Field(id)) => fields.contains(&id),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanKind { Read, Write }

#[derive(Clone, Debug)]
struct LiveLoan {
    id: u32,
    place: StatePlace,
    kind: LoanKind,
}

fn overlap(left: StatePlace, right: StatePlace) -> bool {
    left == StatePlace::Whole || right == StatePlace::Whole || left == right
}

/// An input to the restricted oracle, NOT an authenticated effect summary.
/// The production compiler must derive/seal effects from typed bodies and
/// transitive callees; callers cannot promote this test model to guest ABI.
#[derive(Clone, Debug)]
pub struct EffectModel {
    pub reads: Vec<StatePlace>,
    pub writes: Vec<StatePlace>,
    pub may_suspend: bool,
    pub may_escape: bool,
}

/// A sequential event checker. CFG joins, exceptional edges, loop liveness,
/// cancellation and actual guard lock provenance are outside this model.
pub struct LiveStateLoanModel<'a> {
    layout: &'a StateLayoutModel,
    live: Vec<LiveLoan>,
}

impl<'a> LiveStateLoanModel<'a> {
    pub fn new(layout: &'a StateLayoutModel) -> Self {
        Self { layout, live: Vec::new() }
    }

    fn conflicts(&self, place: StatePlace, kind: LoanKind) -> bool {
        self.live.iter().any(|loan| overlap(loan.place, place)
            && (loan.kind == LoanKind::Write || kind == LoanKind::Write))
    }

    pub fn acquire(&mut self, id: u32, place: StatePlace, kind: LoanKind, pos: Pos) -> Check<()> {
        self.layout.validate_place(place, pos)?;
        if self.live.iter().any(|loan| loan.id == id) {
            return Err(error(pos, "E_BORROW_CONFLICT", "duplicate live loan identity"));
        }
        if self.conflicts(place, kind) {
            return Err(error(pos, "E_BORROW_CONFLICT", "overlapping live state loans"));
        }
        self.live.push(LiveLoan { id, place, kind });
        Ok(())
    }

    pub fn end(&mut self, id: u32, pos: Pos) -> Check<()> {
        if let Some(index) = self.live.iter().position(|loan| loan.id == id) {
            self.live.remove(index);
            Ok(())
        } else {
            Err(error(pos, "E_BORROW", "ending a loan that is not live"))
        }
    }

    pub fn suspend(&self, pos: Pos) -> Check<()> {
        if self.live.is_empty() {
            Ok(())
        } else {
            Err(error(pos, "E_BORROW", "actor-state loan crosses suspension without a frame proof"))
        }
    }

    pub fn check_effect(
        &self, ceiling: &WriteCeiling, effect: Option<&EffectModel>, pos: Pos,
    ) -> Check<()> {
        let effect = effect.ok_or_else(|| error(pos, "E_EFFECT_UNVERIFIED",
            "missing sealed transitive method effects"))?;
        if effect.may_suspend || effect.may_escape {
            return Err(error(pos, "E_BORROW", "unverified suspension or escape in state effect"));
        }
        for place in &effect.reads {
            self.layout.validate_place(*place, pos)?;
            if self.conflicts(*place, LoanKind::Read) {
                return Err(error(pos, "E_BORROW_CONFLICT", "read overlaps live mutable state loan"));
            }
        }
        for place in &effect.writes {
            self.layout.validate_place(*place, pos)?;
            if !self.layout.permits(ceiling, *place) {
                return Err(error(pos, "E_MUTABILITY", "write exceeds declared actor-state ceiling"));
            }
            if self.conflicts(*place, LoanKind::Write) {
                return Err(error(pos, "E_BORROW_CONFLICT", "write overlaps live actor-state loan"));
            }
        }
        Ok(())
    }
}


/// Result of cross-checking a receiver ceiling against actual parsed actor
/// fields. These facts must still be re-authenticated by typed layout and
/// body/effect checking; they do not authorize actor guest compilation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedActorProjection {
    pub actor: String,
    pub method: String,
    pub ceiling: WriteCeiling,
    /// Direct guarded field sites observed in source, NOT a complete effect
    /// summary (unknown callees and transitive effects remain unverified).
    pub observed_fields: BTreeSet<usize>,
    pub observed_writes: BTreeSet<usize>,
}

/// Compiler-path preflight: resolve projected receiver field names against the
/// *actual* actor declaration's fields, not caller-supplied field lists.
/// Absent concrete actor definitions stay unlinked (and guest-blocked) instead
/// of fabricating a layout from the receiver's permission declaration.
pub fn validate_source_actor_state_projections(
    headers: &crate::target_syntax::TargetHeaders,
) -> Check<Vec<ParsedActorProjection>> {
    let mut checked = Vec::new();
    for receiver in &headers.receivers {
        let sites: Vec<_> = headers.state_field_uses.iter()
            .filter(|site| site.owner == receiver.owner &&
                site.method == receiver.name && site.arity == receiver.arity)
            .collect();
        if receiver.state_write.is_none() && sites.is_empty() {
            continue;
        }
        let Some(actor) = headers.definitions.iter().find(|actor| actor.name == receiver.owner)
        else {
            // No implementation or state schema exists yet. Existing typed
            // signature lowering refuses unverified projected receivers.
            continue;
        };
        let names: Vec<&str> = actor.fields.iter().map(|field| field.name.as_str()).collect();
        let schema = StateLayoutModel::new(&actor.name, &names, receiver.pos)?;
        let ceiling = schema.receiver_ceiling(receiver)?;
        if let WriteCeiling::Fields(fields) = &ceiling {
            for id in fields {
                if !actor.fields[*id].binding.rebindable() {
                    return Err(error(receiver.pos, "E_MUTABILITY",
                        format!("actor state field '{}' is not rebindable",
                            actor.fields[*id].name)));
                }
            }
        }
        let mut observed_fields = BTreeSet::new();
        let mut observed_writes = BTreeSet::new();
        for site in sites {
            let StatePlace::Field(id) = schema.place(Some(&site.field), site.pos)? else {
                unreachable!("named field must resolve to a concrete field");
            };
            observed_fields.insert(id);
            if site.write {
                // The parser already rejects critical-read mutations and
                // out-of-projection writes; preserve a second canonical check.
                if !schema.permits(&ceiling, StatePlace::Field(id)) ||
                    !actor.fields[id].binding.rebindable() {
                    return Err(error(site.pos, "E_MUTABILITY",
                        "guarded field write exceeds canonical actor-state permissions"));
                }
                observed_writes.insert(id);
            }
        }
        checked.push(ParsedActorProjection {
            actor: actor.name.clone(),
            method: receiver.name.clone(),
            ceiling,
            observed_fields,
            observed_writes,
        });
    }
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::target_syntax::parse_target_declarations;
    fn p() -> Pos { Pos { line: 1, col: 1 } }
    fn layout() -> StateLayoutModel {
        StateLayoutModel::new("Foo", &["counter", "xyz"], p()).unwrap()
    }
    fn header(receiver: &str) -> ReceiverHeader {
        let src = format!("declare actor Foo; mtd ({receiver}) test(): void {{}}");
        parse_target_declarations(&lex(&src).unwrap()).unwrap().receivers.remove(0)
    }
    fn code<T>(result: Check<T>, expected: &str) {
        assert_eq!(result.err().expect("expected fail-closed diagnostic").code, expected);
    }

    #[test]
    fn canonical_field_identity_and_unknown_schema_fail_closed() {
        let schema = layout();
        assert_eq!(schema.place(Some("counter"), p()).unwrap(), StatePlace::Field(0));
        assert_eq!(schema.place(Some("xyz"), p()).unwrap(), StatePlace::Field(1));
        code(schema.place(Some("missing"), p()), "E_STATE_FIELD_UNKNOWN");
        code(StateLayoutModel::new("Foo", &["counter", "counter"], p()), "E_STATE_FIELD_UNKNOWN");
        code(StateLayoutModel::new("Foo", &[], p()), "E_STATE_FIELD_UNKNOWN");
    }

    #[test]
    fn named_receiver_alpha_equivalence_preserves_write_ceiling() {
        let schema = layout();
        let a = schema.receiver_ceiling(&header("Foo mut self.state.{counter, xyz}")).unwrap();
        let b = schema.receiver_ceiling(&header("Foo mut this.state.{xyz, counter}")).unwrap();
        assert_eq!(a, b);
        assert_eq!(schema.receiver_ceiling(&header("Foo this")).unwrap(),
            WriteCeiling::ReadOnly);
        code(schema.receiver_ceiling(&header("Foo mut self.state.other")), "E_STATE_FIELD_UNKNOWN");
        code(StateLayoutModel::new("Other", &["counter"], p()).unwrap()
            .receiver_ceiling(&header("Foo mut self.state.counter")), "E_ACTOR");
    }

    #[test]
    fn disjoint_live_writers_are_distinct_but_whole_state_conflicts() {
        let schema = layout();
        let mut loans = LiveStateLoanModel::new(&schema);
        loans.acquire(1, StatePlace::Field(0), LoanKind::Write, p()).unwrap();
        loans.acquire(2, StatePlace::Field(1), LoanKind::Write, p()).unwrap();
        code(loans.acquire(3, StatePlace::Whole, LoanKind::Read, p()), "E_BORROW_CONFLICT");
        code(loans.acquire(3, StatePlace::Field(0), LoanKind::Write, p()), "E_BORROW_CONFLICT");
        code(loans.acquire(1, StatePlace::Field(1), LoanKind::Read, p()), "E_BORROW_CONFLICT");
        loans.end(1, p()).unwrap();
        loans.end(2, p()).unwrap();
        loans.acquire(4, StatePlace::Whole, LoanKind::Write, p()).unwrap();
        code(loans.acquire(5, StatePlace::Field(1), LoanKind::Read, p()), "E_BORROW_CONFLICT");
        loans.end(4, p()).unwrap();
        code(loans.end(4, p()), "E_BORROW");
        code(loans.acquire(5, StatePlace::Field(99), LoanKind::Read, p()), "E_STATE_FIELD_UNKNOWN");
    }

    #[test]
    fn shared_reads_coexist_but_block_mutation_until_last_end() {
        let schema = layout();
        let mut loans = LiveStateLoanModel::new(&schema);
        loans.acquire(1, StatePlace::Field(0), LoanKind::Read, p()).unwrap();
        loans.acquire(2, StatePlace::Field(0), LoanKind::Read, p()).unwrap();
        code(loans.acquire(3, StatePlace::Field(0), LoanKind::Write, p()), "E_BORROW_CONFLICT");
        loans.end(1, p()).unwrap();
        code(loans.acquire(3, StatePlace::Field(0), LoanKind::Write, p()), "E_BORROW_CONFLICT");
        loans.end(2, p()).unwrap();
        loans.acquire(3, StatePlace::Field(0), LoanKind::Write, p()).unwrap();
        code(loans.suspend(p()), "E_BORROW");
        loans.end(3, p()).unwrap();
        loans.suspend(p()).unwrap();
    }

    #[test]
    fn unverified_effects_unauthorized_writes_and_escape_fail_closed() {
        let schema = layout();
        let mut loans = LiveStateLoanModel::new(&schema);
        let ceiling = schema.receiver_ceiling(&header("Foo mut this.state.counter")).unwrap();
        code(loans.check_effect(&ceiling, None, p()), "E_EFFECT_UNVERIFIED");
        let mut effect = EffectModel {
            reads: vec![StatePlace::Field(1)], writes: vec![StatePlace::Field(0)],
            may_suspend: false, may_escape: false,
        };
        loans.check_effect(&ceiling, Some(&effect), p()).unwrap();
        effect.writes = vec![StatePlace::Field(1)];
        code(loans.check_effect(&ceiling, Some(&effect), p()), "E_MUTABILITY");
        effect.writes = vec![StatePlace::Field(0)];
        loans.acquire(1, StatePlace::Field(1), LoanKind::Write, p()).unwrap();
        code(loans.check_effect(&ceiling, Some(&effect), p()), "E_BORROW_CONFLICT");
        loans.end(1, p()).unwrap();
        effect.may_suspend = true;
        code(loans.check_effect(&ceiling, Some(&effect), p()), "E_BORROW");
        effect.may_suspend = false;
        effect.may_escape = true;
        code(loans.check_effect(&ceiling, Some(&effect), p()), "E_BORROW");
    }

    #[test]
    fn oracle_does_not_enable_unproven_compiled_actor_guests() {
        let src = "declare actor Foo; mtd (Foo mut self.state.counter) inc(): void {}";
        assert_eq!(crate::compile(src).unwrap_err().code, "E_UNSUPPORTED");
    }
}
