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
    assert!(gate.try_enter_receive(&p).unwrap().is_none());
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

#[test]
fn per_actor_receive_limit_is_bounded_and_retained_until_settlement() {
    let mut g = Gate::<2, 4>::new(Charge::new(2,2,2,2).unwrap());
    for cap in [0, 5, usize::MAX] {
        assert_eq!(g.reserve_with_receive_limit(unit(), cap).unwrap_err(), Error::Policy);
        assert_eq!(g.usage(), [0;4]);
    }
    let p = g.reserve_with_receive_limit(unit(), 1).unwrap();
    assert_eq!(p.generation(), 1);
    let q = g.reserve_with_receive_limit(unit(), 2).unwrap();
    let pi = g.begin_init(&p).unwrap();
    let qi = g.begin_init(&q).unwrap();
    assert_eq!(g.complete_init(&pi, true), Ok(true));
    assert_eq!(g.complete_init(&qi, true), Ok(true));
    let first = g.try_enter_receive(&p).unwrap().unwrap();
    assert!(g.try_enter_receive(&p).unwrap().is_none());
    let x = g.try_enter_receive(&q).unwrap().unwrap();
    let y = g.try_enter_receive(&q).unwrap().unwrap();
    assert!(g.try_enter_receive(&q).unwrap().is_none());
    g.finish_receive(&first).unwrap();
    let replacement = g.try_enter_receive(&p).unwrap().unwrap();
    assert_eq!(g.finish_receive(&first), Err(Error::Stale));
    assert!(g.try_enter_receive(&p).unwrap().is_none());
    g.retire(&p).unwrap();
    assert!(g.try_enter_receive(&p).unwrap().is_none());
    assert_eq!(g.release_after_quiescence(&p), Err(Error::InFlight));
    assert_eq!(g.usage(), [2;4]);
    g.finish_receive(&replacement).unwrap();
    g.release_after_quiescence(&p).unwrap();
    assert_eq!(g.usage(), [1;4]);
    g.retire(&q).unwrap();
    g.finish_receive(&x).unwrap();
    g.finish_receive(&y).unwrap();
    g.release_after_quiescence(&q).unwrap();
    assert_eq!(g.usage(), [0;4]);
    let mut empty = Gate::<1, 0>::new(unit());
    assert_eq!(empty.reserve(unit()).unwrap_err(), Error::Policy);
    assert_eq!(empty.usage(), [0;4]);
}

#[test]
fn concurrent_receive_caps_are_independent_and_retire_fences_new_entries() {
    let gate = Arc::new(Mutex::new(Gate::<2, 4>::new(Charge::new(2,2,2,2).unwrap())));
    let (p,q) = {
        let mut g = gate.lock().unwrap();
        let p = g.reserve_with_receive_limit(unit(), 1).unwrap();
        let q = g.reserve_with_receive_limit(unit(), 3).unwrap();
        for actor in [&p,&q] {
            let lease = g.begin_init(actor).unwrap();
            assert_eq!(g.complete_init(&lease, true), Ok(true));
        }
        (p,q)
    };
    let start = Arc::new(Barrier::new(129));
    let arrived = Arc::new(Barrier::new(129));
    let settle = Arc::new(Barrier::new(129));
    let winners = [AtomicUsize::new(0),AtomicUsize::new(0)];
    std::thread::scope(|s| {
        for i in 0..128 {
            let (gate,start,arrived,settle) = (gate.clone(),start.clone(),arrived.clone(),settle.clone());
            let (actor,winner) = if i%2==0 { (&p,&winners[0]) } else { (&q,&winners[1]) };
            s.spawn(move || {
                start.wait();
                let lease = gate.lock().unwrap().try_enter_receive(actor).unwrap();
                if lease.is_some() { winner.fetch_add(1,Ordering::SeqCst); }
                arrived.wait();
                settle.wait();
                if let Some(lease) = lease {
                    gate.lock().unwrap().finish_receive(&lease).unwrap();
                }
            });
        }
        start.wait();
        arrived.wait();
        let observed = [winners[0].load(Ordering::SeqCst),winners[1].load(Ordering::SeqCst)];
        // Record results before unblocking workers, so a failed assertion does not strand them.
        let retirement = {
            let mut g = gate.lock().unwrap();
            let rp = g.retire(&p);
            let rq = g.retire(&q);
            (rp,rq,g.try_enter_receive(&p).map(|l|l.is_none()),
             g.try_enter_receive(&q).map(|l|l.is_none()),
             g.release_after_quiescence(&p),g.release_after_quiescence(&q),g.usage())
        };
        settle.wait();
        assert_eq!(observed,[1,3]);
        assert_eq!(retirement,(Ok(()),Ok(()),Ok(true),Ok(true),Err(Error::InFlight),Err(Error::InFlight),[2;4]));
    });
    let mut g = gate.lock().unwrap();
    g.release_after_quiescence(&p).unwrap();
    assert_eq!(g.usage(),[1;4]);
    g.release_after_quiescence(&q).unwrap();
    assert_eq!(g.usage(),[0;4]);
}

#[test]
fn concurrent_init_settlement_and_retirement_preserve_quota() {
    let gate = Mutex::new(Gate::<1,1>::new(unit()));
    for _ in 0..2000 {
        let (p,init) = {
            let mut g=gate.lock().unwrap();
            let p=g.reserve(unit()).unwrap();
            let init=g.begin_init(&p).unwrap();
            (p,init)
        };
        let start=Barrier::new(2);
        std::thread::scope(|s| {
            let completion=s.spawn(|| {
                start.wait();
                gate.lock().unwrap().complete_init(&init,true)
            });
            start.wait();
            gate.lock().unwrap().retire(&p).unwrap();
            assert!(completion.join().unwrap().is_ok());
        });
        let mut g=gate.lock().unwrap();
        assert_eq!(g.usage(),[1;4]);
        assert!(!g.may_receive(&p));
        assert!(g.try_enter_receive(&p).unwrap().is_none());
        g.release_after_quiescence(&p).unwrap();
        assert_eq!(g.usage(),[0;4]);
    }
}
