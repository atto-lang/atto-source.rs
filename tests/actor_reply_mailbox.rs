//! Host-side mailbox admission/settlement proofs; no guest ABI claimed.
use atto_compiler::actor_reply_mailbox::{MailFault, UnaryMailbox};
use atto_compiler::actor_reply_slot::{ReplyFailure, ReplyOutcome, SlotFault};

#[test]
fn admitted_request_allocates_both_queue_entry_and_reply_slot() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(2, 2, 1);
    assert_eq!(m.pending(), 0);
    let receipt = m.admit(41, 20, 100, 1).unwrap();
    assert_eq!(m.pending(), 1);
    assert_eq!(m.reply_slots_free(), 1);
    assert_eq!(m.poll(&receipt), Ok(None));
    let delivery = m.next(1).unwrap();
    assert_eq!(*delivery.payload(), 41);
    assert_eq!(m.pending(), 0);
    assert_eq!(m.reply_slots_free(), 1);
    m.stage(&delivery, 42, 2).unwrap();
    assert_eq!(m.poll(&receipt), Ok(None));
    m.complete(delivery, 3).unwrap();
    assert_eq!(m.collect(&receipt), Ok(ReplyOutcome::Value(42)));
    assert_eq!(m.reply_slots_free(), 2);
}

#[test]
fn queue_backpressure_never_leaks_a_reply_slot() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 3, 1);
    let r = m.admit(1, 1, 100, 1).unwrap();
    assert_eq!(m.admit(2, 1, 100, 1), Err(MailFault::QueueFull));
    assert_eq!(m.pending(), 1);
    assert_eq!(m.reply_slots_free(), 2);
    m.cancel(&r).unwrap();
    m.collect(&r).unwrap();
    assert_eq!(m.reply_slots_free(), 3);
}

#[test]
fn slot_backpressure_includes_inflight_turns() {
    let mut m: UnaryMailbox<i32, i32> = UnaryMailbox::new(3, 1, 1);
    let r = m.admit(12, 1, 100, 1).unwrap();
    let delivery = m.next(1).unwrap();
    assert_eq!(m.admit(42, 1, 100, 1), Err(MailFault::Reply(SlotFault::Full)));
    assert_eq!(m.pending(), 0);
    m.stage(&delivery, 13, 2).unwrap();
    m.complete(delivery, 3).unwrap();
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Value(13)));
    assert!(m.admit(42, 1, 100, 4).is_ok());
}

#[test]
fn duplicate_write_cannot_overwrite_staged_value() {
    let mut m = UnaryMailbox::new(1, 1, 1);
    let r = m.admit(0, 1, 90, 1).unwrap();
    let d = m.next(1).unwrap();
    m.stage(&d, 8, 2).unwrap();
    assert_eq!(m.stage(&d, 9, 3), Err(SlotFault::AlreadyStaged));
    m.complete(d, 4).unwrap();
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Value(8)));
}

#[test]
fn handler_panic_discards_staged_value_and_settles_with_failure() {
    let mut m = UnaryMailbox::new(1, 1, 1);
    let r = m.admit(0, 1, 90, 1).unwrap();
    let d = m.next(1).unwrap();
    m.stage(&d, 8, 2).unwrap();
    m.fail(d).unwrap();
    assert_eq!(m.collect(&r),
        Ok(ReplyOutcome::Failure(ReplyFailure::HandlerFailed)));
}

#[test]
fn successful_turn_without_value_is_terminal_missing_reply() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let r = m.admit(0, 1, 90, 1).unwrap();
    let delivery = m.next(1).unwrap();
    m.complete(delivery, 2).unwrap();
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Failure(ReplyFailure::MissingReply)));
}

#[test]
fn cancellation_before_dispatch_skips_queued_message() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(2, 2, 1);
    let r = m.admit(0, 1, 90, 1).unwrap();
    m.cancel(&r).unwrap();
    assert!(m.next(1).is_none());
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
}

#[test]
fn cancellation_during_turn_rejects_late_completion() {
    let mut m = UnaryMailbox::new(1, 1, 1);
    let r = m.admit(0, 1, 90, 1).unwrap();
    let d = m.next(1).unwrap();
    m.cancel(&r).unwrap();
    assert_eq!(m.stage(&d, 3, 2), Err(SlotFault::AlreadySettled));
    assert_eq!(m.complete(d, 3), Err(SlotFault::AlreadySettled));
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
}

#[test]
fn timeout_skips_queued_and_prevents_late_delivery() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(2, 2, 1);
    let queued = m.admit(4, 1, 4, 1).unwrap();
    let next = m.admit(5, 1, 50, 1).unwrap();
    assert_eq!(m.expire(4), 1);
    let only = m.next(4).unwrap();
    assert_eq!(*only.payload(), 5);
    assert!(m.next(1).is_none());
    assert_eq!(m.collect(&queued), Ok(ReplyOutcome::Failure(ReplyFailure::TimedOut)));
    m.stage(&only, 6, 10).unwrap();
    m.complete(only, 11).unwrap();
    assert_eq!(m.collect(&next), Ok(ReplyOutcome::Value(6)));
}

#[test]
fn actor_retirement_fences_old_permit_and_allows_new_incarnation() {
    let mut m = UnaryMailbox::new(2, 2, 10);
    let r = m.admit(42, 2, 100, 1).unwrap();
    let d = m.next(1).unwrap();
    m.stage(&d, 44, 2).unwrap();
    assert_eq!(m.retire(11), Ok(1));
    assert!(m.is_closed());
    assert_eq!(m.admit(99, 2, 100, 3), Err(MailFault::Closed));
    assert_eq!(m.complete(d, 3), Err(SlotFault::AlreadySettled));
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Failure(ReplyFailure::ActorRetired)));
    m.reopen();
    let fresh = m.admit(99, 2, 100, 4).unwrap();
    let d = m.next(4).unwrap();
    m.stage(&d, 101, 5).unwrap();
    m.complete(d, 6).unwrap();
    assert_eq!(m.collect(&fresh), Ok(ReplyOutcome::Value(101)));
}

#[test]
fn actor_retirement_cancels_queued_requests_without_running_them() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(3, 3, 1);
    let a = m.admit(1, 1, 90, 1).unwrap();
    let b = m.admit(2, 1, 90, 1).unwrap();
    assert_eq!(m.retire(2), Ok(2));
    assert_eq!(m.pending(), 0);
    assert!(m.next(1).is_none());
    assert_eq!(m.collect(&a), Ok(ReplyOutcome::Failure(ReplyFailure::ActorRetired)));
    assert_eq!(m.collect(&b), Ok(ReplyOutcome::Failure(ReplyFailure::ActorRetired)));
}

#[test]
fn independent_callers_complete_out_of_order_without_correlation_crossing() {
    let mut m = UnaryMailbox::new(2, 2, 1);
    let first = m.admit(10, 1, 100, 1).unwrap();
    let second = m.admit(20, 2, 100, 1).unwrap();
    let a = m.next(1).unwrap();
    let b = m.next(1).unwrap();
    m.stage(&b, 200, 2).unwrap();
    m.complete(b, 3).unwrap();
    assert_eq!(m.poll(&first), Ok(None));
    assert_eq!(m.poll(&second), Ok(Some(&ReplyOutcome::Value(200))));
    m.stage(&a, 100, 4).unwrap();
    m.complete(a, 5).unwrap();
    assert_eq!(m.collect(&second), Ok(ReplyOutcome::Value(200)));
    assert_eq!(m.collect(&first), Ok(ReplyOutcome::Value(100)));
}

#[test]
fn invalid_deadline_does_not_enqueue_or_reserve() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    assert_eq!(m.admit(1, 1, 10, 10), Err(MailFault::Reply(SlotFault::InvalidDeadline)));
    assert_eq!(m.pending(), 0);
    assert_eq!(m.reply_slots_free(), 1);
}

#[test]
fn dispatcher_refuses_expired_request_without_prior_timer_tick() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(2, 2, 1);
    let receipt = m.admit(7, 2, 10, 1).unwrap();
    assert!(m.next(10).is_none());
    assert_eq!(m.collect(&receipt),
        Ok(ReplyOutcome::Failure(ReplyFailure::TimedOut)));
}
#[test]
fn late_stage_rejects_success_and_settles_timeout_without_timer_tick() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let receipt = m.admit(7, 2, 10, 1).unwrap();
    let delivery = m.next(2).unwrap();
    assert_eq!(m.stage(&delivery, 9, 10), Err(SlotFault::AlreadySettled));
    assert_eq!(m.complete(delivery, 11), Err(SlotFault::AlreadySettled));
    assert_eq!(m.collect(&receipt),
        Ok(ReplyOutcome::Failure(ReplyFailure::TimedOut)));
}
#[test]
fn cancelled_queued_request_reclaims_backpressure_before_dispatch() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 2, 1);
    let cancelled = m.admit(7, 2, 50, 1).unwrap();
    assert_eq!(m.admit(8, 2, 50, 2), Err(MailFault::QueueFull));
    m.cancel(&cancelled).unwrap();
    let fresh = m.admit(8, 2, 50, 3).unwrap();
    assert_eq!(*m.next(3).unwrap().payload(), 8);
    assert_eq!(m.collect(&cancelled),
        Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
    assert_eq!(m.reply_slots_free(), 1);
    m.cancel(&fresh).unwrap();
    assert_eq!(m.collect(&fresh),
        Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
}

#[test]
fn identical_incarnations_in_distinct_mailboxes_do_not_share_authority() {
    let mut a: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let mut b: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let ra = a.admit(11, 7, 100, 1).unwrap();
    let rb = b.admit(22, 7, 100, 1).unwrap();
    assert_ne!(ra, rb);
    let da = a.next(1).unwrap();
    let db = b.next(1).unwrap();
    assert_eq!(b.poll(&ra), Err(SlotFault::Stale));
    assert_eq!(b.cancel(&ra), Err(SlotFault::Stale));
    assert_eq!(b.stage(&da, 999, 101), Err(SlotFault::Stale));
    assert_eq!(b.poll(&rb), Ok(None));
    a.stage(&da, 111, 2).unwrap();
    b.stage(&db, 222, 2).unwrap();
    a.complete(da, 3).unwrap();
    b.complete(db, 3).unwrap();
    assert_eq!(a.collect(&ra), Ok(ReplyOutcome::Value(111)));
    assert_eq!(b.collect(&rb), Ok(ReplyOutcome::Value(222)));
}

#[test]
fn dropped_mailbox_cannot_authorize_replacement_with_same_counters() {
    let (old_receipt, old_delivery) = {
        let mut old: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
        let receipt = old.admit(11, 7, 100, 1).unwrap();
        (receipt, old.next(1).unwrap())
    };
    let mut fresh: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let r = fresh.admit(22, 7, 100, 1).unwrap();
    let d = fresh.next(1).unwrap();
    assert_eq!(fresh.complete(old_delivery, 2), Err(SlotFault::Stale));
    assert_eq!(fresh.collect(&old_receipt), Err(SlotFault::Stale));
    assert_eq!(fresh.poll(&r), Ok(None));
    fresh.stage(&d, 42, 3).unwrap();
    fresh.complete(d, 4).unwrap();
    assert_eq!(fresh.collect(&r), Ok(ReplyOutcome::Value(42)));
}

#[test]
fn foreign_failure_does_not_settle_local_request() {
    let mut a: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let mut b: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let ra = a.admit(1, 7, 100, 1).unwrap();
    let rb = b.admit(2, 7, 100, 1).unwrap();
    assert_eq!(b.fail(a.next(1).unwrap()), Err(SlotFault::Stale));
    assert_eq!(b.poll(&rb), Ok(None));
    a.cancel(&ra).unwrap();
    b.cancel(&rb).unwrap();
    assert_eq!(a.collect(&ra), Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
    assert_eq!(b.collect(&rb), Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
}

#[test]
fn late_or_settled_delivery_cannot_expire_unrelated_slots() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(2, 2, 1);
    let a = m.admit(1, 1, 10, 1).unwrap();
    let b = m.admit(2, 1, 20, 1).unwrap();
    let da = m.next(1).unwrap();
    let db = m.next(1).unwrap();
    assert_eq!(m.stage(&da, 99, 20), Err(SlotFault::AlreadySettled));
    assert_eq!(m.poll(&a), Ok(Some(&ReplyOutcome::Failure(ReplyFailure::TimedOut))));
    assert_eq!(m.poll(&b), Ok(None));
    assert_eq!(m.stage(&da, 99, 21), Err(SlotFault::AlreadySettled));
    assert_eq!(m.poll(&b), Ok(None));
    assert_eq!(m.expire(21), 1);
    assert_eq!(m.complete(db, 21), Err(SlotFault::AlreadySettled));
    assert_eq!(m.collect(&a), Ok(ReplyOutcome::Failure(ReplyFailure::TimedOut)));
    assert_eq!(m.collect(&b), Ok(ReplyOutcome::Failure(ReplyFailure::TimedOut)));
}

#[test]
fn premature_collection_preserves_receipt_and_reclaims_capacity_after_completion() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let r = m.admit(7, 1, 100, 1).unwrap();
    assert_eq!(m.collect(&r), Err(SlotFault::NotSettled));
    assert_eq!(m.reply_slots_free(), 0);
    let d = m.next(2).unwrap();
    m.stage(&d, 42, 3).unwrap();
    assert_eq!(m.collect(&r), Err(SlotFault::NotSettled));
    m.complete(d, 4).unwrap();
    assert_eq!(m.collect(&r), Ok(ReplyOutcome::Value(42)));
    assert_eq!(m.collect(&r), Err(SlotFault::Stale));
    assert_eq!(m.reply_slots_free(), 1);
    let fresh = m.admit(8, 1, 100, 5).unwrap();
    assert_eq!(m.collect(&r), Err(SlotFault::Stale));
    assert_eq!(m.poll(&fresh), Ok(None));
    m.cancel(&fresh).unwrap();
    assert_eq!(m.collect(&fresh), Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
    assert_eq!(m.reply_slots_free(), 1);
}

#[test]
fn foreign_collection_does_not_destroy_caller_receipt() {
    let mut a: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let mut b: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let ra = a.admit(1, 1, 100, 1).unwrap();
    let rb = b.admit(2, 1, 100, 1).unwrap();
    assert_eq!(b.collect(&ra), Err(SlotFault::Stale));
    assert_eq!(b.poll(&rb), Ok(None));
    a.cancel(&ra).unwrap();
    assert_eq!(a.collect(&ra), Ok(ReplyOutcome::Failure(ReplyFailure::Cancelled)));
    assert_eq!(a.reply_slots_free(), 1);
    b.expire(100);
    assert_eq!(b.collect(&rb), Ok(ReplyOutcome::Failure(ReplyFailure::TimedOut)));
    assert_eq!(b.reply_slots_free(), 1);
}

#[test]
fn abandoning_queued_request_reclaims_both_budgets_and_fences_receipt() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let old = m.admit(10, 20, 100, 1).unwrap();
    assert_eq!(m.abandon(&old), Ok(()));
    assert_eq!(m.pending(), 0);
    assert_eq!(m.reply_slots_free(), 1);
    assert_eq!(m.poll(&old), Err(SlotFault::Stale));
    assert_eq!(m.abandon(&old), Err(SlotFault::Stale));
    let next = m.admit(11, 20, 100, 2).unwrap();
    let delivery = m.next(2).unwrap();
    assert_eq!(*delivery.payload(), 11);
    m.stage(&delivery, 42, 3).unwrap();
    m.complete(delivery, 4).unwrap();
    assert_eq!(m.collect(&next), Ok(ReplyOutcome::Value(42)));
}

#[test]
fn abandoning_inflight_staged_request_rejects_late_permit_after_reuse() {
    let mut m: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let old = m.admit(1, 20, 100, 1).unwrap();
    let delivery = m.next(1).unwrap();
    m.stage(&delivery, 90, 2).unwrap();
    m.abandon(&old).unwrap();
    let next = m.admit(2, 20, 100, 3).unwrap();
    assert_eq!(m.stage(&delivery, 99, 3), Err(SlotFault::Stale));
    assert_eq!(m.complete(delivery, 3), Err(SlotFault::Stale));
    assert_eq!(m.poll(&next), Ok(None));
    let current = m.next(3).unwrap();
    m.stage(&current, 42, 4).unwrap();
    m.complete(current, 5).unwrap();
    assert_eq!(m.collect(&next), Ok(ReplyOutcome::Value(42)));
}

#[test]
fn foreign_abandon_does_not_remove_local_queue_and_settled_abandon_reclaims() {
    let mut a: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let mut b: UnaryMailbox<i64, i64> = UnaryMailbox::new(1, 1, 1);
    let foreign = a.admit(1, 20, 100, 1).unwrap();
    let local = b.admit(2, 20, 100, 1).unwrap();
    assert_eq!(b.abandon(&foreign), Err(SlotFault::Stale));
    assert_eq!(b.pending(), 1);
    assert_eq!(b.reply_slots_free(), 0);
    let turn = b.next(1).unwrap();
    b.stage(&turn, 42, 2).unwrap();
    b.complete(turn, 3).unwrap();
    assert_eq!(b.abandon(&local), Ok(()));
    assert_eq!(b.collect(&local), Err(SlotFault::Stale));
    assert_eq!(b.reply_slots_free(), 1);
    assert_eq!(a.poll(&foreign), Ok(None));
}
