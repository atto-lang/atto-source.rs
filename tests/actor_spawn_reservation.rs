use atto_compiler::actor_spawn_reservation::{Charge, Error, Gate};
use std::sync::{Arc, Barrier, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

fn unit() -> Charge { Charge::new(1, 1, 1, 1).unwrap() }

#[test]
fn lifecycle_is_fenced_and_replay_safe() {
    let mut gate = Gate::<1, 2>::new(unit());
    let mut foreign = Gate::<1, 2>::new(unit());
    let p = gate.reserve(unit()).unwrap();
    assert!(!gate.may_receive(&p));
    assert!(gate.try_enter_receive(&p).unwrap().is_none());
    assert_eq!(foreign.begin_init(&p).unwrap_err(), Error::Stale);
    let init = gate.begin_init(&p).unwrap();
    assert_eq!(gate.begin_init(&p).unwrap_err(), Error::InitAuthority);
    assert_eq!(gate.complete_init(&init, true), Ok(true));
    assert_eq!(gate.complete_init(&init, true), Err(Error::InitAuthority));
    let a = gate.try_enter_receive(&p).unwrap().unwrap();
    let b = gate.try_enter_receive(&p).unwrap().unwrap();
    assert_eq!(gate.try_enter_receive(&p).unwrap_err(), Error::Full);
    assert_eq!(gate.release_after_quiescence(&p), Err(Error::NotRetired));
    gate.retire(&p).unwrap();
    assert!(!gate.may_receive(&p));
    assert!(gate.try_enter_receive(&p).unwrap().is_none());
    assert_eq!(gate.release_after_quiescence(&p), Err(Error::InFlight));
    gate.finish_receive(&a).unwrap();
    assert_eq!(gate.finish_receive(&a), Err(Error::Stale));
    assert_eq!(gate.release_after_quiescence(&p), Err(Error::InFlight));
    gate.finish_receive(&b).unwrap();
    gate.release_after_quiescence(&p).unwrap();
    assert_eq!(gate.usage(), [0;4]);
    let q = gate.reserve(unit()).unwrap();
    assert!(q.generation() > p.generation());
    assert_eq!(gate.retire(&p), Err(Error::Stale));
    let qi = gate.begin_init(&q).unwrap();
    gate.retire(&q).unwrap();
    assert_eq!(gate.release_after_quiescence(&q), Err(Error::InFlight));
    assert_eq!(gate.complete_init(&qi, true), Ok(false));
    gate.release_after_quiescence(&q).unwrap();
}

#[test]
fn independent_limits_overflow_and_rollback() {
    let mut gate = Gate::<1, 1>::new(unit());
    for c in [[2,1,1,1],[1,2,1,1],[1,1,2,1],[1,1,1,2]] {
        assert_eq!(gate.reserve(Charge::new(c[0],c[1],c[2],c[3]).unwrap()).unwrap_err(), Error::Quota);
        assert_eq!(gate.usage(), [0;4]);
    }
    assert_eq!(Charge::new(0,1,1,1), Err(Error::Quota));
    for _ in 0..100_000 {
        let p = gate.reserve(unit()).unwrap();
        let init = gate.begin_init(&p).unwrap();
        assert_eq!(gate.complete_init(&init, false), Ok(false));
        gate.release_after_quiescence(&p).unwrap();
    }
    assert_eq!(gate.usage(), [0;4]);
    let max = Charge::new(u64::MAX,u64::MAX,u64::MAX,u64::MAX).unwrap();
    let mut large = Gate::<2, 1>::new(max);
    let p = large.reserve(max).unwrap();
    assert_eq!(large.reserve(unit()).unwrap_err(), Error::Quota);
    large.retire(&p).unwrap();
    large.release_after_quiescence(&p).unwrap();
    assert_eq!(large.usage(), [0;4]);
}

#[test]
fn concurrent_host_admission_never_overbooks() {
    let gate = Arc::new(Mutex::new(Gate::<1,1>::new(unit())));
    let start = Arc::new(Barrier::new(32));
    let attempted = Arc::new(Barrier::new(32));
    let winners = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        for _ in 0..32 {
            let (gate, start, attempted, winners) = (gate.clone(), start.clone(), attempted.clone(), winners.clone());
            s.spawn(move || {
                start.wait();
                let result = gate.lock().unwrap().reserve(unit());
                if result.is_ok() { winners.fetch_add(1, Ordering::SeqCst); }
                attempted.wait();
                match result {
                    Ok(p) => {
                        let mut g = gate.lock().unwrap();
                        g.retire(&p).unwrap(); g.release_after_quiescence(&p).unwrap();
                    }
                    Err(e) => assert_eq!(e, Error::Quota),
                }
            });
        }
    });
    assert_eq!(winners.load(Ordering::SeqCst), 1);
    assert_eq!(gate.lock().unwrap().usage(), [0;4]);
}
