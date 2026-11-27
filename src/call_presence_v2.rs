//! Typed positional call planning for the V2 omission contract.
//! Not yet wired to source lowering. No default expression is evaluated here.
use crate::ast::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Formal {
    pub ty: Type,
    pub omittable: bool,
    pub none_default: bool,
    pub omission_default: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Typed(Type),
    UntypedNone,
    Hole,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Supplied(usize),
    ExplicitHole(usize),
    Absent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    KeepSupplied,
    CheckOuterNone,
    UseNoneDefault,
    UseOmissionDefault,
    SynthesizeNone,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub origin: Origin,
    pub action: Action,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub candidate: usize,
    pub slots: Vec<Slot>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidFormal,
    DuplicateSignature,
    NoMatch,
    Ambiguous,
}
fn option(ty: Type) -> bool {
    matches!(ty, Type::OptionInt | Type::OptionStr | Type::OptionResultIntStr)
}
fn validate(formals: &[Formal]) -> Result<(), Error> {
    for f in formals {
        if (f.none_default && !option(f.ty))
            || (f.omission_default && !f.omittable)
            || (f.omittable && !option(f.ty) && !f.omission_default)
        {
            return Err(Error::InvalidFormal);
        }
    }
    Ok(())
}
fn bind(formals: &[Formal], items: &[Item]) -> Option<Vec<Slot>> {
    if items.len() > formals.len() {
        return None;
    }
    formals.iter().enumerate().map(|(i, f)| {
        let (origin, action) = match items.get(i) {
            Some(Item::Typed(t)) if *t == f.ty => (
                Origin::Supplied(i),
                if f.none_default { Action::CheckOuterNone } else { Action::KeepSupplied },
            ),
            Some(Item::UntypedNone) if option(f.ty) => (
                Origin::Supplied(i),
                if f.none_default { Action::UseNoneDefault } else { Action::KeepSupplied },
            ),
            Some(Item::Hole) | None if f.omittable => (
                if i < items.len() { Origin::ExplicitHole(i) } else { Origin::Absent },
                if f.omission_default { Action::UseOmissionDefault } else { Action::SynthesizeNone },
            ),
            _ => return None,
        };
        Some(Slot { origin, action })
    }).collect()
}
/// Candidates must have the same resolved owner/name and already-resolved
/// concrete types. No labels, generics, variadics or implicit conversions.
/// Reject duplicate nominal signatures before inspecting calls. Exact complete
/// arity wins among type-compatible candidates; otherwise require uniqueness.
pub fn select(candidates: &[Vec<Formal>], items: &[Item]) -> Result<Plan, Error> {
    for (i, a) in candidates.iter().enumerate() {
        validate(a)?;
        if candidates[..i].iter().any(|b| a.iter().map(|f| f.ty).eq(b.iter().map(|f| f.ty))) {
            return Err(Error::DuplicateSignature);
        }
    }
    let mut winner: Option<(bool, Plan)> = None;
    let mut ambiguous = false;
    for (candidate, formals) in candidates.iter().enumerate() {
        let Some(slots) = bind(formals, items) else { continue };
        let exact = slots.iter().all(|s| matches!(s.origin, Origin::Supplied(_)));
        let plan = Plan { candidate, slots };
        match &winner {
            None => { winner = Some((exact, plan)); ambiguous = false; }
            Some((false, _)) if exact => { winner = Some((true, plan)); ambiguous = false; }
            Some((rank, _)) if *rank == exact => ambiguous = true,
            _ => {}
        }
    }
    if ambiguous { return Err(Error::Ambiguous); }
    winner.map(|(_, plan)| plan).ok_or(Error::NoMatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(ty: Type, omittable: bool, none_default: bool, omission_default: bool) -> Formal {
        Formal { ty, omittable, none_default, omission_default }
    }
    #[test]
    fn distinct_original_origins_and_actions() {
        let c = vec![vec![f(Type::OptionInt, true, true, true)]];
        for (items, origin, action) in [
            (vec![], Origin::Absent, Action::UseOmissionDefault),
            (vec![Item::Hole], Origin::ExplicitHole(0), Action::UseOmissionDefault),
            (vec![Item::UntypedNone], Origin::Supplied(0), Action::UseNoneDefault),
            (vec![Item::Typed(Type::OptionInt)], Origin::Supplied(0), Action::CheckOuterNone),
        ] {
            assert_eq!(select(&c, &items).unwrap().slots, vec![Slot { origin, action }]);
        }
        assert_eq!(select(&c, &[Item::Typed(Type::Int)]), Err(Error::NoMatch));
        let c = vec![vec![f(Type::OptionInt, true, true, false)]];
        assert_eq!(select(&c, &[]).unwrap().slots[0].action, Action::SynthesizeNone);
    }
    #[test]
    fn complete_arity_and_type_compatibility() {
        let short = vec![f(Type::Int, false, false, false)];
        let long = vec![short[0], f(Type::Int, true, false, true)];
        for c in [vec![short.clone(), long.clone()], vec![long.clone(), short.clone()]] {
            let p = select(&c, &[Item::Typed(Type::Int)]).unwrap();
            assert_eq!(c[p.candidate].len(), 1);
        }
        let c = vec![vec![f(Type::Bool, false, false, false)], long];
        assert_eq!(select(&c, &[Item::Typed(Type::Int)]).unwrap().candidate, 1);
    }
    #[test]
    fn ambiguity_and_duplicates_are_order_independent() {
        let a = vec![f(Type::OptionInt, true, true, true)];
        let b = vec![f(Type::OptionResultIntStr, true, false, false)];
        for c in [vec![a.clone(), b.clone()], vec![b, a.clone()]] {
            assert_eq!(select(&c, &[]), Err(Error::Ambiguous));
            assert_eq!(select(&c, &[Item::Hole]), Err(Error::Ambiguous));
            assert_eq!(select(&c, &[Item::UntypedNone]), Err(Error::Ambiguous));
            let p = select(&c, &[Item::Typed(Type::OptionInt)]).unwrap();
            assert_eq!(c[p.candidate][0].ty, Type::OptionInt);
        }
        let other = vec![f(Type::OptionInt, false, false, false)];
        assert_eq!(select(&[a, other], &[]), Err(Error::DuplicateSignature));
    }
    #[test]
    fn required_slots_and_invalid_formals_fail_closed() {
        let c = vec![vec![f(Type::Int, true, false, true), f(Type::Int, false, false, false)]];
        assert_eq!(select(&c, &[Item::Typed(Type::Int)]), Err(Error::NoMatch));
        let p = select(&c, &[Item::Hole, Item::Typed(Type::Int)]).unwrap();
        assert_eq!(p.slots[1].origin, Origin::Supplied(1));
        for formal in [
            f(Type::Int, true, false, false),
            f(Type::Int, false, true, false),
            f(Type::Int, false, false, true),
        ] {
            assert_eq!(select(&[vec![formal]], &[]), Err(Error::InvalidFormal));
        }
    }
}
