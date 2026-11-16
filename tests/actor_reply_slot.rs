//! HOST_ONLY bounded one-shot correlation registry conformance. No Atto guest.
use atto_compiler::actor_reply_slot::{ReplyFailure as Failure, ReplyOutcome as Outcome,
    ReplyRegistry, SlotFault as Fault};

#[test]
fn stage_is_not_observable_until_turn_commits() {
    let mut l = ReplyRegistry::new(1, 7);
    let t = l.admit(10, 100, 1).unwrap();
    l.stage(&t, 42, 2).unwrap();
    assert_eq!(l.poll(&t), Ok(None));
    assert_eq!(l.completed_count(), 0);
    l.finish_turn(&t, 3).unwrap();
    assert_eq!(l.completed_count(), 1);
    assert_eq!(l.receive(&t), Ok(Outcome::Value(42)));
    assert_eq!(l.completed_count(), 0);
    assert_eq!(l.receive(&t), Err(Fault::Stale));
}
#[test]
fn returned_value_and_explicit_reply_share_one_permission() {
    let mut l = ReplyRegistry::new(1, 1);
    let t = l.admit(2, 50, 1).unwrap();
    l.stage(&t, 10, 2).unwrap();
    assert_eq!(l.stage(&t, 20, 2), Err(Fault::AlreadyStaged));
    l.finish_turn(&t, 3).unwrap();
    assert_eq!(l.stage(&t, 30, 3), Err(Fault::AlreadySettled));
    assert_eq!(l.finish_turn(&t, 3), Err(Fault::AlreadySettled));
    assert_eq!(l.receive(&t), Ok(Outcome::Value(10)));
}
#[test]
fn panic_discards_staged_reply_instead_of_success_plus_error() {
    let mut l = ReplyRegistry::new(1, 1);
    let t = l.admit(1, 100, 1).unwrap();
    l.stage(&t, 40, 3).unwrap();
    l.fail_turn(&t).unwrap();
    assert_eq!(l.receive(&t), Ok(Outcome::Failure(Failure::HandlerFailed)));
}
#[test]
fn missing_response_always_settles() {
    let mut l: ReplyRegistry<i64> = ReplyRegistry::new(1, 1);
    let t = l.admit(1, 100, 1).unwrap();
    l.finish_turn(&t, 4).unwrap();
    assert_eq!(l.receive(&t), Ok(Outcome::Failure(Failure::MissingReply)));
}
#[test]
fn cancelled_reply_wins_over_late_return() {
    let mut l = ReplyRegistry::new(1, 1);
    let t = l.admit(1, 100, 1).unwrap();
    l.stage(&t, 42, 2).unwrap();
    l.cancel(&t).unwrap();
    assert_eq!(l.stage(&t, 24, 2), Err(Fault::AlreadySettled));
    assert_eq!(l.finish_turn(&t, 3), Err(Fault::AlreadySettled));
    assert_eq!(l.receive(&t), Ok(Outcome::Failure(Failure::Cancelled)));
}
#[test]
fn timeout_expires_active_slots_and_rejects_late_reply() {
    let mut l = ReplyRegistry::new(2, 1);
    let a = l.admit(1, 30, 1).unwrap();
    let b = l.admit(1, 40, 1).unwrap();
    l.stage(&a, 2, 10).unwrap();
    l.stage(&b, 3, 10).unwrap();
    assert_eq!(l.expire(30), 1);
    assert_eq!(l.stage(&a, 5, 31), Err(Fault::AlreadySettled));
    assert_eq!(l.receive(&a), Ok(Outcome::Failure(Failure::TimedOut)));
    l.finish_turn(&b, 35).unwrap();
    assert_eq!(l.receive(&b), Ok(Outcome::Value(3)));
}
#[test]
fn late_commit_at_deadline_is_timeout_not_success() {
    let mut l = ReplyRegistry::new(1, 1);
    let t = l.admit(1, 50, 1).unwrap();
    l.stage(&t, 42, 10).unwrap();
    l.finish_turn(&t, 50).unwrap();
    assert_eq!(l.receive(&t), Ok(Outcome::Failure(Failure::TimedOut)));
}
#[test]
fn retire_actor_invalidates_staged_result_but_preserves_error_receipt() {
    let mut l = ReplyRegistry::new(1, 3);
    let a = l.admit(10, 60, 1).unwrap();
    l.stage(&a, 42, 2).unwrap();
    assert_eq!(l.retire_actor(4), Ok(1));
    assert_eq!(l.stage(&a, 50, 3), Err(Fault::AlreadySettled));
    assert_eq!(l.receive(&a), Ok(Outcome::Failure(Failure::ActorRetired)));
    let b = l.admit(10, 60, 4).unwrap();
    assert_eq!(l.stage(&a, 99, 4), Err(Fault::Stale));
    l.stage(&b, 44, 4).unwrap();
    l.finish_turn(&b, 5).unwrap();
    assert_eq!(l.receive(&b), Ok(Outcome::Value(44)));
}
#[test]
fn concurrent_correlation_out_of_order_and_capacity_bound() {
    let mut l = ReplyRegistry::new(2, 1);
    assert_eq!(l.capacity(), 2);
    let a = l.admit(8, 80, 1).unwrap();
    let b = l.admit(9, 80, 1).unwrap();
    assert_eq!(l.available(), 0);
    assert_eq!(l.admit(9, 80, 1), Err(Fault::Full));
    l.stage(&b, 22, 2).unwrap();
    l.finish_turn(&b, 3).unwrap();
    l.stage(&a, 11, 4).unwrap();
    l.finish_turn(&a, 5).unwrap();
    assert_eq!(l.receive(&a), Ok(Outcome::Value(11)));
    assert_eq!(l.receive(&b), Ok(Outcome::Value(22)));
    assert_eq!(l.available(), 2);
}
#[test]
fn a_failed_admission_does_not_consume_a_slot() {
    let mut l: ReplyRegistry<i32> = ReplyRegistry::new(1, 1);
    assert_eq!(l.admit(1, 4, 4), Err(Fault::InvalidDeadline));
    assert_eq!(l.available(), 1);
    let t = l.admit(1, 5, 4).unwrap();
    assert_eq!(l.available(), 0);
    l.cancel(&t).unwrap();
    assert_eq!(l.available(), 0);
    assert_eq!(l.receive(&t), Ok(Outcome::Failure(Failure::Cancelled)));
    assert_eq!(l.available(), 1);
}
#[test]
fn duplicate_terminal_cannot_be_settled_twice() {
    let mut l: ReplyRegistry<i64> = ReplyRegistry::new(1, 1);
    let t = l.admit(10, 20, 1).unwrap();
    l.fail_turn(&t).unwrap();
    assert_eq!(l.fail_turn(&t), Err(Fault::AlreadySettled));
    assert_eq!(l.cancel(&t), Err(Fault::AlreadySettled));
    assert_eq!(l.receive(&t), Ok(Outcome::Failure(Failure::HandlerFailed)));
}
#[test]
fn retired_generations_cannot_go_backward() {
    let mut l: ReplyRegistry<i64> = ReplyRegistry::new(1, 9);
    assert_eq!(l.retire_actor(9), Err(Fault::Stale));
    assert_eq!(l.retire_actor(8), Err(Fault::Stale));
    assert_eq!(l.retire_actor(10), Ok(0));
}
