use atto_borrow_checker::actor_state_guard_v1::{GuardError, Mode};
use atto_borrow_checker::actor_state_runtime_v1::{GuardedActorRegion, RuntimeError};
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;

#[test]
fn five_mail_counter_writes_are_exclusive_and_ordered() {
    let state = GuardedActorRegion::new(4, 0_i64).unwrap();
    let mut values = Vec::new();
    for (actor, next) in (1_u64..=5).zip(1_i64..=5) {
        state.request(actor, 4, Mode::Write).unwrap();
        let scopes = state.admit_ready().unwrap();
        assert_eq!(scopes.len(), 1);
        let total = state.execute_write(scopes.into_iter().next().unwrap(), |s| {
            *s += next;
            *s
        });
        values.push(total.unwrap());
        assert_eq!(state.active_count(), Ok(0));
    }
    assert_eq!(values, vec![1, 3, 6, 10, 15]);
}

#[test]
fn two_readers_are_physically_concurrent_and_writer_cannot_barge() {
    let state = GuardedActorRegion::new(2, 42_i64).unwrap();
    state.request(1, 2, Mode::Read).unwrap();
    state.request(2, 2, Mode::Read).unwrap();
    let scopes = state.admit_ready().unwrap();
    assert_eq!(scopes.len(), 2);
    let barrier = Arc::new(Barrier::new(3));
    let (entered_tx, entered_rx) = mpsc::channel();
    let handles: Vec<_> = scopes.into_iter().map(|scope| {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        let entered_tx = entered_tx.clone();
        thread::spawn(move || {
            state.execute_read(scope, |v| {
                entered_tx.send(()).unwrap();
                barrier.wait();
                *v
            }).unwrap()
        })
    }).collect();
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    state.request(3, 2, Mode::Write).unwrap();
    state.request(4, 2, Mode::Read).unwrap();
    assert!(state.admit_ready().unwrap().is_empty());
    assert_eq!(state.active_count(), Ok(2));
    barrier.wait();
    for h in handles {
        assert_eq!(h.join().unwrap(), 42);
    }
    let writer = state.admit_ready().unwrap();
    assert_eq!(writer.len(), 1);
    assert_eq!(state.execute_write(writer.into_iter().next().unwrap(), |v| {
        *v += 1;
        *v
    }), Ok(43));
    let later_reader = state.admit_ready().unwrap();
    assert_eq!(later_reader.len(), 1);
    assert_eq!(state.execute_read(later_reader.into_iter().next().unwrap(), |v| *v), Ok(43));
}

#[test]
fn read_scope_cannot_be_escalated_into_write() {
    let state = GuardedActorRegion::new(3, 7_i64).unwrap();
    state.request(8, 3, Mode::Read).unwrap();
    let scope = state.admit_ready().unwrap().pop().unwrap();
    assert_eq!(
        state.execute_write(scope, |v| *v = 9),
        Err(RuntimeError::WrongMode)
    );
    assert_eq!(state.active_count(), Ok(0));
    state.request(8, 3, Mode::Read).unwrap();
    let scope = state.admit_ready().unwrap().pop().unwrap();
    assert_eq!(state.execute_read(scope, |v| *v), Ok(7));
}

#[test]
fn wrong_region_cannot_use_a_permit_to_read_other_state() {
    let first = GuardedActorRegion::new(5, 17_i64).unwrap();
    let second = GuardedActorRegion::new(5, 99_i64).unwrap();
    first.request(1, 5, Mode::Read).unwrap();
    let scope = first.admit_ready().unwrap().pop().unwrap();
    assert_eq!(second.execute_read(scope, |v| *v), Err(RuntimeError::ForeignRegion));
    assert_eq!(first.active_count(), Ok(0));
    second.request(1, 5, Mode::Read).unwrap();
    let scope = second.admit_ready().unwrap().pop().unwrap();
    assert_eq!(second.execute_read(scope, |v| *v), Ok(99));
}

#[test]
fn retirement_waits_for_real_in_flight_reader() {
    let state = GuardedActorRegion::new(8, 5_i64).unwrap();
    state.request(11, 8, Mode::Read).unwrap();
    let scope = state.admit_ready().unwrap().pop().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let barrier = Arc::new(Barrier::new(2));
    let b = Arc::clone(&barrier);
    let state_for_reader = Arc::clone(&state);
    let reader = thread::spawn(move || state_for_reader.execute_read(scope, |v| {
        entered_tx.send(()).unwrap();
        b.wait();
        *v
    }).unwrap());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    state.retire(9).unwrap();
    assert_eq!(state.may_reclaim(), Ok(false));
    assert_eq!(state.request(12, 8, Mode::Write), Err(RuntimeError::Guard(GuardError::StaleGeneration)));
    assert_eq!(state.request(12, 9, Mode::Write), Err(RuntimeError::Guard(GuardError::Retired)));
    barrier.wait();
    assert_eq!(reader.join().unwrap(), 5);
    assert_eq!(state.active_count(), Ok(0));
    assert_eq!(state.may_reclaim(), Ok(true));
}

#[test]
fn stale_granted_scope_is_rejected_before_callback_execution() {
    let state = GuardedActorRegion::new(1, 13_i64).unwrap();
    state.request(2, 1, Mode::Write).unwrap();
    let scope = state.admit_ready().unwrap().pop().unwrap();
    state.retire(2).unwrap();
    let invoked = std::cell::Cell::new(false);
    assert_eq!(
        state.execute_write(scope, |v| {
            invoked.set(true);
            *v = 0;
        }),
        Err(RuntimeError::Guard(GuardError::StaleGeneration))
    );
    assert!(!invoked.get());
    assert_eq!(state.may_reclaim(), Ok(true));
}

#[test]
fn cancellation_drops_pending_but_does_not_revoke_admitted_holder() {
    let state = GuardedActorRegion::new(12, 3_i64).unwrap();
    let first = state.request(1, 12, Mode::Write).unwrap();
    let second = state.request(2, 12, Mode::Read).unwrap();
    let writer = state.admit_ready().unwrap().pop().unwrap();
    assert_eq!(state.cancel_pending(first, 1, 12), Err(RuntimeError::Guard(GuardError::UnknownTicket)));
    assert_eq!(state.cancel_pending(second, 1, 12), Err(RuntimeError::Guard(GuardError::WrongHolder)));
    state.cancel_pending(second, 2, 12).unwrap();
    assert_eq!(state.execute_write(writer, |v| { *v *= 2; *v }), Ok(6));
    assert!(state.admit_ready().unwrap().is_empty());
    assert_eq!(state.active_count(), Ok(0));
}

#[test]
fn unwinding_releases_logical_permit_and_poisoned_state_fails_closed() {
    let state = GuardedActorRegion::new(15, 0_i64).unwrap();
    state.request(1, 15, Mode::Write).unwrap();
    let scope = state.admit_ready().unwrap().pop().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), RuntimeError> = state.execute_write(scope, |_| panic!("guest callback"));
    }));
    assert!(result.is_err());
    assert_eq!(state.active_count(), Ok(0));
    state.request(2, 15, Mode::Read).unwrap();
    let scope = state.admit_ready().unwrap().pop().unwrap();
    assert_eq!(state.execute_read(scope, |v| *v), Err(RuntimeError::Poisoned));
    state.retire(16).unwrap();
    assert_eq!(state.may_reclaim(), Ok(true));
}
