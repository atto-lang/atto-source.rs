use oreslang_source_rs::owner_arena::{
    ArenaError, OwnerArena, MAX_ARENA_BYTES, MAX_OFFSET_ALIGNMENT,
};

#[test]
fn bump_allocation_is_bounded_and_non_moving() {
    let mut arena = OwnerArena::new(64).unwrap();
    let first = arena.allocate(3, 1).unwrap();
    let second = arena.allocate(8, 8).unwrap();
    assert_eq!(arena.capacity(), 64);
    assert_eq!(arena.used_bytes(), 16);
    assert_eq!(arena.remaining_bytes(), 48);
    arena.write(first).unwrap().copy_from_slice(&[7, 8, 9]);
    arena.write(second).unwrap().copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(arena.read(first).unwrap(), &[7, 8, 9]);
    assert_eq!(arena.read(second).unwrap(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    // Adding another object never relocates a previous handle.
    let third = arena.allocate(8, 8).unwrap();
    assert_eq!(arena.read(first).unwrap(), &[7, 8, 9]);
    assert_eq!(arena.read(third).unwrap(), &[0; 8]);
    assert_eq!(arena.used_bytes(), 24);
}

#[test]
fn foreign_arena_handles_fail_even_with_equal_offsets() {
    let mut one = OwnerArena::new(8).unwrap();
    let mut two = OwnerArena::new(8).unwrap();
    let h1 = one.allocate(4, 1).unwrap();
    let h2 = two.allocate(4, 1).unwrap();
    one.write(h1).unwrap().copy_from_slice(&[1, 2, 3, 4]);
    two.write(h2).unwrap().copy_from_slice(&[5, 6, 7, 8]);
    assert_eq!(one.read(h2), Err(ArenaError::ForeignArena));
    assert_eq!(two.read(h1), Err(ArenaError::ForeignArena));
    assert_eq!(one.write(h2), Err(ArenaError::ForeignArena));
    assert_eq!(one.read(h1).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(two.read(h2).unwrap(), &[5, 6, 7, 8]);
}

#[test]
fn reset_invalidates_old_handles_and_reinitializes_bytes() {
    let mut arena = OwnerArena::new(16).unwrap();
    let old = arena.allocate(16, 1).unwrap();
    arena.write(old).unwrap().fill(0xAA);
    arena.reset().unwrap();
    assert_eq!(arena.used_bytes(), 0);
    assert_eq!(arena.read(old), Err(ArenaError::StaleHandle));
    assert_eq!(arena.write(old), Err(ArenaError::StaleHandle));
    let fresh = arena.allocate(16, 1).unwrap();
    assert_eq!(arena.read(fresh).unwrap(), &[0; 16]);
    assert_ne!(old, fresh);
}

#[test]
fn failures_do_not_change_cursor_or_valid_existing_bytes() {
    let mut arena = OwnerArena::new(16).unwrap();
    let original = arena.allocate(8, 8).unwrap();
    arena.write(original).unwrap().fill(0x5A);
    assert_eq!(arena.allocate(9, 1), Err(ArenaError::OutOfSpace));
    assert_eq!(arena.allocate(0, 1), Err(ArenaError::InvalidAllocation));
    assert_eq!(arena.allocate(1, 0), Err(ArenaError::InvalidAlignment));
    assert_eq!(arena.allocate(1, 3), Err(ArenaError::InvalidAlignment));
    assert_eq!(
        arena.allocate(1, MAX_OFFSET_ALIGNMENT * 2),
        Err(ArenaError::InvalidAlignment)
    );
    assert_eq!(arena.used_bytes(), 8);
    assert_eq!(arena.read(original).unwrap(), &[0x5A; 8]);
    let final_handle = arena.allocate(8, 1).unwrap();
    assert_eq!(arena.read(final_handle).unwrap(), &[0; 8]);
    assert_eq!(arena.remaining_bytes(), 0);
}

#[test]
fn allocation_padding_is_cleared_after_reset() {
    let mut arena = OwnerArena::new(64).unwrap();
    let leading = arena.allocate(1, 1).unwrap();
    let padded = arena.allocate(16, 16).unwrap();
    arena.write(leading).unwrap().fill(0xA5);
    arena.write(padded).unwrap().fill(0xB6);
    assert_eq!(arena.used_bytes(), 32);
    arena.reset().unwrap();
    let all = arena.allocate(32, 1).unwrap();
    assert_eq!(arena.read(all).unwrap(), &[0; 32]);
}

#[test]
fn empty_and_oversized_arena_admission_is_explicit() {
    let mut empty = OwnerArena::new(0).unwrap();
    assert_eq!(empty.capacity(), 0);
    assert_eq!(empty.allocate(1, 1), Err(ArenaError::OutOfSpace));
    assert!(matches!(
        OwnerArena::new(MAX_ARENA_BYTES + 1),
        Err(ArenaError::InvalidCapacity)
    ));
}

#[test]
fn concurrent_owned_arenas_never_share_handles_or_storage() {
    let threads: Vec<_> = (0..8)
        .map(|index| {
            std::thread::spawn(move || {
                let mut arena = OwnerArena::new(64).unwrap();
                let handle = arena.allocate(8, 8).unwrap();
                arena.write(handle).unwrap().fill(index as u8);
                let out = arena.read(handle).unwrap().to_vec();
                arena.reset().unwrap();
                assert_eq!(arena.read(handle), Err(ArenaError::StaleHandle));
                out
            })
        })
        .collect();
    for (index, thread) in threads.into_iter().enumerate() {
        assert_eq!(thread.join().unwrap(), vec![index as u8; 8]);
    }
}

#[test]
fn reset_and_allocation_loops_remain_fixed_capacity() {
    let mut arena = OwnerArena::new(128).unwrap();
    for epoch in 0..1000_u64 {
        let first = arena.allocate(3, 1).unwrap();
        let second = arena.allocate(16, 16).unwrap();
        arena.write(first).unwrap().fill(epoch as u8);
        arena.write(second).unwrap().fill(0xF1);
        assert_eq!(arena.used_bytes(), 32);
        arena.reset().unwrap();
        assert_eq!(arena.used_bytes(), 0);
        assert_eq!(arena.read(second), Err(ArenaError::StaleHandle));
    }
}

#[test]
fn retired_arena_never_reopens_and_invalidates_every_handle() {
    let mut arena = OwnerArena::new(64).unwrap();
    let h1 = arena.allocate(8, 8).unwrap();
    arena.write(h1).unwrap().fill(0xD2);
    assert!(!arena.is_retired());
    arena.retire();
    assert!(arena.is_retired());
    assert_eq!(arena.used_bytes(), 0);
    assert_eq!(arena.read(h1), Err(ArenaError::Retired));
    assert_eq!(arena.write(h1), Err(ArenaError::Retired));
    assert_eq!(arena.reset(), Err(ArenaError::Retired));
    assert_eq!(arena.allocate(8, 8), Err(ArenaError::Retired));
    arena.retire();
    assert!(arena.is_retired());
    assert_eq!(arena.capacity(), 64);
}

#[test]
fn replacement_arena_never_accepts_dropped_arena_handles() {
    let h1 = {
        let mut first = OwnerArena::new(8).unwrap();
        first.allocate(8, 1).unwrap()
    };
    let mut next = OwnerArena::new(8).unwrap();
    let fresh = next.allocate(8, 1).unwrap();
    assert_eq!(next.read(h1), Err(ArenaError::ForeignArena));
    assert_ne!(h1, fresh);
}
