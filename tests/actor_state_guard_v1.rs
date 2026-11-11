use atto_borrow_checker::actor_state_guard_v1::{GuardError, GuardQueue, Mode, MAX_OUTSTANDING};

#[test]
fn contiguous_readers_share_and_late_writer_is_exclusive() {
    let mut q = GuardQueue::new(7).unwrap();
    q.request(1, 7, Mode::Read).unwrap();
    q.request(2, 7, Mode::Read).unwrap();
    let readers = q.admit_ready();
    assert_eq!(readers.len(), 2);
    assert_eq!(q.active_count(), 2);
    for (actor, guard) in [(1, &readers[0]), (2, &readers[1])] {
        assert_eq!(q.access(guard, actor, false), Ok(()));
        assert_eq!(q.access(guard, actor, true), Err(GuardError::WriteRequiresExclusive));
    }
    q.request(3, 7, Mode::Write).unwrap();
    q.request(4, 7, Mode::Read).unwrap();
    assert!(q.admit_ready().is_empty());
    q.release(&readers[0], 1).unwrap();
    assert!(q.admit_ready().is_empty());
    q.release(&readers[1], 2).unwrap();
    let writer = q.admit_ready();
    assert_eq!(writer.len(), 1);
    assert_eq!(writer[0].actor(), 3);
    assert_eq!(writer[0].mode(), Mode::Write);
    assert_eq!(q.access(&writer[0], 3, true), Ok(()));
    assert!(q.admit_ready().is_empty());
    q.release(&writer[0], 3).unwrap();
    let later_reader = q.admit_ready();
    assert_eq!(later_reader.len(), 1);
    assert_eq!(later_reader[0].actor(), 4);
}

#[test]
fn writer_ahead_of_readers_does_not_allow_barging() {
    let mut q = GuardQueue::new(10).unwrap();
    q.request(5, 10, Mode::Write).unwrap();
    q.request(6, 10, Mode::Read).unwrap();
    q.request(7, 10, Mode::Read).unwrap();
    let grant = q.admit_ready();
    assert_eq!(grant.len(), 1);
    assert_eq!(grant[0].actor(), 5);
    assert!(q.admit_ready().is_empty());
    q.release(&grant[0], 5).unwrap();
    let readers = q.admit_ready();
    assert_eq!(readers.len(), 2);
    assert_eq!(readers[0].actor(), 6);
    assert_eq!(readers[1].actor(), 7);
}

#[test]
fn cancellation_never_cancels_a_live_writer() {
    let mut q = GuardQueue::new(4).unwrap();
    let first = q.request(8, 4, Mode::Write).unwrap();
    let second = q.request(9, 4, Mode::Read).unwrap();
    let active = q.admit_ready();
    assert_eq!(active[0].ticket(), first);
    assert_eq!(q.cancel_pending(first, 8, 4), Err(GuardError::UnknownTicket));
    assert_eq!(q.cancel_pending(second, 8, 4), Err(GuardError::WrongHolder));
    q.cancel_pending(second, 9, 4).unwrap();
    assert_eq!(q.waiting_count(), 0);
    q.release(&active[0], 8).unwrap();
    assert!(q.admit_ready().is_empty());
}

#[test]
fn wrong_holder_double_release_and_reentrancy_fail_closed() {
    let mut q = GuardQueue::new(22).unwrap();
    let id = q.request(1, 22, Mode::Write).unwrap();
    assert_eq!(q.request(1, 22, Mode::Read), Err(GuardError::Reentrant));
    let grant = q.admit_ready();
    assert_eq!(grant[0].ticket(), id);
    assert_eq!(q.request(1, 22, Mode::Read), Err(GuardError::Reentrant));
    assert_eq!(q.access(&grant[0], 2, true), Err(GuardError::WrongHolder));
    assert_eq!(q.release(&grant[0], 2), Err(GuardError::WrongHolder));
    q.release(&grant[0], 1).unwrap();
    assert_eq!(q.release(&grant[0], 1), Err(GuardError::UnknownTicket));
    assert_eq!(q.access(&grant[0], 1, false), Err(GuardError::UnknownTicket));
}

#[test]
fn hot_upgrade_fences_all_access_but_retains_physical_live_guards() {
    let mut q = GuardQueue::new(12).unwrap();
    q.request(3, 12, Mode::Read).unwrap();
    q.request(4, 12, Mode::Read).unwrap();
    let permits = q.admit_ready();
    q.request(5, 12, Mode::Write).unwrap();
    assert!(!q.may_reclaim());
    q.retire(13).unwrap();
    assert_eq!(q.waiting_count(), 0);
    assert_eq!(q.active_count(), 2);
    assert_eq!(q.access(&permits[0], 3, false), Err(GuardError::StaleGeneration));
    assert_eq!(q.release(&permits[0], 3), Err(GuardError::PhysicalQuiescenceRequired));
    assert_eq!(q.request(6, 12, Mode::Read), Err(GuardError::StaleGeneration));
    assert_eq!(q.request(6, 13, Mode::Read), Err(GuardError::Retired));
    assert!(!q.may_reclaim());
    q.complete_physical_quiescence(&permits[0], 3).unwrap();
    assert!(!q.may_reclaim());
    q.complete_physical_quiescence(&permits[1], 4).unwrap();
    assert!(q.may_reclaim());
    assert_eq!(q.complete_physical_quiescence(&permits[1], 4), Err(GuardError::UnknownTicket));
}

#[test]
fn invalid_epoch_cannot_claim_reclamation_or_flush_live_access() {
    let mut q = GuardQueue::new(9).unwrap();
    q.request(1, 9, Mode::Write).unwrap();
    let p = q.admit_ready();
    assert_eq!(q.retire(9), Err(GuardError::StaleGeneration));
    assert_eq!(q.retire(8), Err(GuardError::StaleGeneration));
    assert_eq!(q.complete_physical_quiescence(&p[0], 1), Err(GuardError::NotRetired));
    assert_eq!(q.access(&p[0], 1, true), Ok(()));
}

#[test]
fn outstanding_budget_rejects_more_than_one_thousand_and_twenty_four() {
    let mut q = GuardQueue::new(1).unwrap();
    for actor in 1..=MAX_OUTSTANDING as u64 {
        q.request(actor, 1, Mode::Read).unwrap();
    }
    assert_eq!(q.request(1_025, 1, Mode::Read), Err(GuardError::Capacity));
    let permits = q.admit_ready();
    assert_eq!(permits.len(), MAX_OUTSTANDING);
    assert_eq!(q.request(1_025, 1, Mode::Read), Err(GuardError::Capacity));
    q.release(&permits[0], 1).unwrap();
    assert!(q.request(1_025, 1, Mode::Read).is_ok());
}

#[test]
fn invalid_id_and_epoch_never_grant_permission() {
    assert_eq!(GuardQueue::new(0).unwrap_err(), GuardError::InvalidIdentity);
    let mut q = GuardQueue::new(2).unwrap();
    assert_eq!(q.request(0, 2, Mode::Read), Err(GuardError::InvalidIdentity));
    assert_eq!(q.request(1, 0, Mode::Read), Err(GuardError::InvalidIdentity));
    assert_eq!(q.request(1, 1, Mode::Read), Err(GuardError::StaleGeneration));
    assert_eq!(q.active_count(), 0);
    assert_eq!(q.waiting_count(), 0);
}
