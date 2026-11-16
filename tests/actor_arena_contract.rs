use oreslang_source_rs::actor_arena::{ArenaError, FixedActorArena};

#[derive(Debug, Eq, PartialEq)]
struct OwnedValue {
    count: u64,
}

#[test]
fn move_in_mutate_and_move_out_without_gc() {
    let mut arena = FixedActorArena::<OwnedValue, 2>::new(10, 4).unwrap();
    assert!(arena.is_empty());
    assert_eq!(arena.capacity(), 2);
    let handle = arena.insert(OwnedValue { count: 7 }).unwrap();
    assert_eq!(handle.actor_id(), 10);
    assert_eq!(handle.incarnation(), 4);
    assert_eq!(arena.get(handle).unwrap().count, 7);
    arena.get_mut(handle).unwrap().count = 42;
    assert_eq!(arena.take(handle), Ok(OwnedValue { count: 42 }));
    assert!(arena.is_empty());
}

#[test]
fn stale_handle_is_rejected_after_reuse() {
    let mut arena = FixedActorArena::<u32, 1>::new(10, 2).unwrap();
    let stale = arena.insert(5).unwrap();
    assert_eq!(arena.take(stale), Ok(5));
    let fresh = arena.insert(9).unwrap();
    assert_eq!(stale.slot(), fresh.slot());
    assert_ne!(stale.generation(), fresh.generation());
    assert_eq!(arena.get(stale), Err(ArenaError::StaleGeneration));
    assert_eq!(arena.get_mut(stale), Err(ArenaError::StaleGeneration));
    assert_eq!(arena.take(stale), Err(ArenaError::StaleGeneration));
    assert_eq!(arena.get(fresh), Ok(&9));
}

#[test]
fn cross_actor_handle_is_rejected() {
    let mut owner = FixedActorArena::<u32, 1>::new(123, 1).unwrap();
    let handle = owner.insert(42).unwrap();
    let other = FixedActorArena::<u32, 1>::new(456, 1).unwrap();
    assert_eq!(other.get(handle), Err(ArenaError::WrongOwner));
}

#[test]
fn restart_fences_old_incarnation() {
    let mut before = FixedActorArena::<u64, 1>::new(123, 77).unwrap();
    let old = before.insert(42).unwrap();
    let mut after = FixedActorArena::<u64, 1>::new(123, 78).unwrap();
    let fresh = after.insert(99).unwrap();
    assert_eq!(after.get(old), Err(ArenaError::WrongIncarnation));
    assert_eq!(after.get(fresh), Ok(&99));
}

#[test]
fn bounded_capacity_is_enforced_without_eviction() {
    let mut arena = FixedActorArena::<u64, 2>::new(10, 1).unwrap();
    let a = arena.insert(10).unwrap();
    let b = arena.insert(20).unwrap();
    assert_eq!(arena.len(), 2);
    assert_eq!(arena.insert(30), Err(ArenaError::CapacityExhausted));
    assert_eq!(arena.get(a), Ok(&10));
    assert_eq!(arena.get(b), Ok(&20));
    assert_eq!(arena.take(a), Ok(10));
    let c = arena.insert(30).unwrap();
    assert_eq!(arena.get(c), Ok(&30));
    assert_eq!(arena.get(b), Ok(&20));
}

#[test]
fn checked_owner_identity_is_nonzero() {
    assert!(matches!(
        FixedActorArena::<u8, 1>::new(0, 1),
        Err(ArenaError::InvalidOwner)
    ));
    assert!(matches!(
        FixedActorArena::<u8, 1>::new(1, 0),
        Err(ArenaError::InvalidOwner)
    ));
}

#[test]
fn independently_owned_slots_do_not_relocate_on_take() {
    let mut arena = FixedActorArena::<u64, 3>::new(88, 1).unwrap();
    let a = arena.insert(10).unwrap();
    let b = arena.insert(20).unwrap();
    let c = arena.insert(30).unwrap();
    assert_eq!(arena.take(b), Ok(20));
    assert_eq!(arena.get(a), Ok(&10));
    assert_eq!(arena.get(c), Ok(&30));
    assert_eq!(a.slot(), 0);
    assert_eq!(c.slot(), 2);
}

#[test]
fn duplicate_take_is_denied() {
    let mut arena = FixedActorArena::<u64, 1>::new(2, 3).unwrap();
    let handle = arena.insert(42).unwrap();
    assert_eq!(arena.take(handle), Ok(42));
    assert_eq!(arena.take(handle), Err(ArenaError::StaleGeneration));
}

#[test]
fn handle_cannot_claim_access_after_dropped_owner() {
    let stale = {
        let mut first = FixedActorArena::<u64, 1>::new(2, 1).unwrap();
        first.insert(42).unwrap()
    };
    let mut restarted = FixedActorArena::<u64, 1>::new(2, 2).unwrap();
    let _ = restarted.insert(99).unwrap();
    assert_eq!(restarted.get(stale), Err(ArenaError::WrongIncarnation));
}
