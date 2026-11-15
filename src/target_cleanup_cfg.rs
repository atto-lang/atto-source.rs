//! Executable native cleanup-CFG prototype for *validated frontend lease plans*.
//!
//! This is a deliberately separate experimental layer: it does NOT emit mtd
//! methods into native/Wasm guest code or establish actor scheduler semantics.
//! Verified paths are derived from real source-located lease obligations.
//! This module's proof is NOT accepted by require_backend_cleanup_proof; that
//! production gate remains closed until actual backend code is inspected.
use std::collections::BTreeSet;

use crate::diag::{Diagnostic, Pos};
use crate::target_cleanup::{
    LeaseCleanupExit, ReceiverCleanupPlan, StateLeaseMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupExit {
    Normal,
    CheckedPanic,
    CooperativeCancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupOp {
    Acquire { lease: usize, mode: StateLeaseMode },
    Release { lease: usize },
    Exit(CleanupExit),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupPath {
    pub lease: usize,
    pub exit: CleanupExit,
    pub pos: Pos,
    pub ops: Vec<CleanupOp>,
}

/// No public constructor for the checked CFG; only the path verifier below
/// can construct it. This is an *experimental path witness*, not the
/// production backend release proof needed to enable actor receiver lowering.
#[derive(Clone, Debug)]
pub struct VerifiedCleanupCfg {
    paths: Vec<CleanupPath>,
    leases: usize,
}

impl VerifiedCleanupCfg {
    pub fn paths(&self) -> &[CleanupPath] { &self.paths }
    pub fn lease_count(&self) -> usize { self.leases }
    /// A native runtime operation is admitted only for an actual verified
    /// path of the expected lease, mode, and exit kind.
    fn permits(&self, id:usize, mode:StateLeaseMode, exit:CleanupExit)->bool {
        self.paths.iter().any(|p| p.lease == id && p.exit == exit
            && matches!(&p.ops[..],
                [CleanupOp::Acquire { lease, mode: m },
                 CleanupOp::Release { lease: r },
                 CleanupOp::Exit(e)]
                if *lease == id && *r == id && *m == mode && *e == exit))
    }
}

fn fail(pos: Pos, why: &'static str) -> Diagnostic {
    Diagnostic::new(pos, "E_CLEANUP", why)
}

/// Verify each concrete three-instruction cleanup path, and verify the set
/// includes exactly one normal/cancel path per lease and exactly one checked
/// panic path when the callback contains potentially trapping arithmetic.
/// The graph has no loops or branches *within* a path: other forms are not
/// admitted by this prototype and must not be treated as checked.
fn verify_paths(
    paths: &[CleanupPath], expected: &[(StateLeaseMode, bool)],
) -> Result<(), Diagnostic> {
    let mut seen = BTreeSet::new();
    for path in paths {
        let Some((mode, checked)) = expected.get(path.lease) else {
            return Err(fail(path.pos, "cleanup path refers to nonexistent lease"));
        };
        if path.exit == CleanupExit::CheckedPanic && !*checked {
            return Err(fail(path.pos, "unnecessary checked-panic cleanup edge"));
        }
        if !seen.insert((path.lease, path.exit as u8)) {
            return Err(fail(path.pos, "duplicate lease exit cleanup edge"));
        }
        match path.ops.as_slice() {
            [CleanupOp::Acquire { lease: a, mode: got },
             CleanupOp::Release { lease: r },
             CleanupOp::Exit(exit)]
                if a == &path.lease && r == &path.lease
                    && got == mode && exit == &path.exit => {},
            _ => return Err(fail(path.pos,
                "cleanup CFG requires exactly one matched acquire/release before exit")),
        }
    }
    for (id,(_,checked)) in expected.iter().enumerate() {
        for exit in [CleanupExit::Normal, CleanupExit::CooperativeCancel] {
            if !seen.contains(&(id,exit as u8)) {
                return Err(fail(Pos::default(),
                    "missing normal or cancellation lease cleanup path"));
            }
        }
        if *checked && !seen.contains(&(id,CleanupExit::CheckedPanic as u8)) {
            return Err(fail(Pos::default(),
                "missing checked-panic cleanup path"));
        }
    }
    Ok(())
}

/// Lower a source-checked frontend plan into a strict, executable native
/// proof-of-concept cleanup CFG. It rejects forged frontend proof bits and
/// missing/duplicate obligations, but does not confer production AOT safety.
pub fn lower_verified_cleanup_cfg(
    plan: &ReceiverCleanupPlan,
) -> Result<VerifiedCleanupCfg, Diagnostic> {
    if plan.backend_cleanup_proven {
        return Err(fail(Pos::default(),
            "frontend backend-proof flags may not authorize code generation"));
    }
    let mut paths = Vec::new();
    let mut expected = Vec::new();
    for (id,lease) in plan.leases.iter().enumerate() {
        if lease.lease != id || lease.receiver != plan.receiver {
            return Err(fail(lease.acquire_pos, "lease identity is inconsistent"));
        }
        if lease.cleanup.len() != 1 + usize::from(lease.checked_arithmetic) {
            return Err(fail(lease.acquire_pos, "missing or duplicated lease cleanup obligation"));
        }
        let Some(normal) = lease.cleanup.first() else {
            return Err(fail(lease.acquire_pos, "missing normal cleanup obligation"));
        };
        if normal.lease != id || normal.exit != LeaseCleanupExit::Normal
            || normal.backend_release_proven {
            return Err(fail(lease.acquire_pos, "invalid normal lease cleanup obligation"));
        }
        if lease.checked_arithmetic {
            let panic = &lease.cleanup[1];
            if panic.lease != id || panic.exit != LeaseCleanupExit::CheckedPanic
                || panic.backend_release_proven {
                return Err(fail(lease.acquire_pos, "invalid checked-panic cleanup obligation"));
            }
        }
        expected.push((lease.mode.clone(), lease.checked_arithmetic));
        let path = |exit: CleanupExit, pos: Pos| CleanupPath {
            lease: id, exit, pos,
            ops: vec![
                CleanupOp::Acquire { lease: id, mode: lease.mode.clone() },
                CleanupOp::Release { lease: id },
                CleanupOp::Exit(exit),
            ],
        };
        paths.push(path(CleanupExit::Normal, normal.pos));
        if lease.checked_arithmetic {
            paths.push(path(CleanupExit::CheckedPanic, lease.cleanup[1].pos));
        }
        // Cooperative cancellation is observed only at a safe instruction
        // boundary; a held lease must be released before returning Cancel.
        paths.push(path(CleanupExit::CooperativeCancel, lease.normal_end_pos));
    }
    verify_paths(&paths,&expected)?;
    Ok(VerifiedCleanupCfg { paths, leases: plan.leases.len() })
}

/// Native executable witness using Rust's lock guards and unwinding. This is
/// not the language runtime; its purpose is to exercise observable release
/// behavior with the same strict cleanup paths recorded above.
#[cfg(not(target_arch = "wasm32"))]
pub mod native_experiment {
    use super::{CleanupExit, StateLeaseMode, VerifiedCleanupCfg};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError,
    };

    pub struct LeaseCell {
        value: RwLock<i64>,
        releases: AtomicUsize,
    }

    struct SharedGuard<'a> {
        lock: Option<RwLockReadGuard<'a,i64>>,
        releases: &'a AtomicUsize,
    }
    impl Drop for SharedGuard<'_> {
        fn drop(&mut self) {
            // Unlock *before* reporting the release. Incrementing while the
            // guard field is still alive makes release_count observable too
            // early from a second thread.
            drop(self.lock.take());
            self.releases.fetch_add(1,Ordering::SeqCst);
        }
    }

    struct ExclusiveGuard<'a> {
        lock: Option<RwLockWriteGuard<'a,i64>>,
        releases: &'a AtomicUsize,
    }
    impl Drop for ExclusiveGuard<'_> {
        fn drop(&mut self) {
            drop(self.lock.take());
            self.releases.fetch_add(1,Ordering::SeqCst);
        }
    }

    impl LeaseCell {
        pub fn new(initial:i64) -> Self {
            Self { value:RwLock::new(initial),releases:AtomicUsize::new(0) }
        }
        /// Only experimental Rust code calls this, NOT emitted Atto receiver code.
        /// Fixed read operation with no user callback. The only local
        /// observable exit after acquisition is normal return; unlike an
        /// arbitrary callback this does not require a panic cleanup edge.
        pub fn read_snapshot(&self, cfg:&VerifiedCleanupCfg,
            lease:usize) -> Result<i64,&'static str> {
            if !cfg.permits(lease,StateLeaseMode::SharedRead,CleanupExit::Normal) {
                return Err("unverified shared-read cleanup path");
            }
            let guard=SharedGuard {
                lock:Some(self.value.read().unwrap_or_else(|err|err.into_inner())),
                releases:&self.releases,
            };
            Ok(**guard.lock.as_ref().expect("live read lease"))
        }

        /// Arbitrary native callbacks can panic, even when their Rust body
        /// looks pure. They require both the normal AND unwind cleanup paths.
        /// This does not authorize callbacks in the generated Atto backend.
        pub fn with_shared<R>(&self, cfg:&VerifiedCleanupCfg, lease:usize,
            f:impl FnOnce(i64)->R) -> Result<R,&'static str> {
            if !cfg.permits(lease,StateLeaseMode::SharedRead,CleanupExit::Normal)
                || !cfg.permits(lease,StateLeaseMode::SharedRead,CleanupExit::CheckedPanic) {
                return Err("shared callback requires verified normal and panic cleanup paths");
            }
            let guard=SharedGuard {
                lock:Some(self.value.read().unwrap_or_else(|err|err.into_inner())),
                releases:&self.releases,
            };
            Ok(f(**guard.lock.as_ref().expect("live read lease")))
        }
        pub fn checked_add(&self, cfg:&VerifiedCleanupCfg, lease:usize,
            delta:i64) -> Result<std::thread::Result<i64>,&'static str> {
            if !cfg.permits(lease,StateLeaseMode::ExclusiveWrite,CleanupExit::Normal)
                || !cfg.permits(lease,StateLeaseMode::ExclusiveWrite,CleanupExit::CheckedPanic) {
                return Err("missing verified exclusive checked-arithmetic cleanup path");
            }
            Ok(catch_unwind(AssertUnwindSafe(|| {
                let mut guard=ExclusiveGuard {
                    lock:Some(self.value.write().unwrap_or_else(|err|err.into_inner())),
                    releases:&self.releases,
                };
                let state=guard.lock.as_mut().expect("live write lease");
                let value=state.checked_add(delta)
                    .expect("checked actor-state arithmetic overflow");
                **state=value;
                value
            })))
        }
        pub fn cancel_while_exclusive(&self, cfg:&VerifiedCleanupCfg,
            lease:usize) -> Result<(), &'static str> {
            if !cfg.permits(lease,StateLeaseMode::ExclusiveWrite,CleanupExit::CooperativeCancel) {
                return Err("unverified cooperative cancellation cleanup path");
            }
            let _guard=ExclusiveGuard {
                lock:Some(self.value.write().unwrap_or_else(|err|err.into_inner())),
                releases:&self.releases,
            };
            Err("cancelled")
        }
        pub fn panic_while_shared(&self, cfg:&VerifiedCleanupCfg,
            lease:usize) -> Result<std::thread::Result<()>,&'static str> {
            if !cfg.permits(lease,StateLeaseMode::SharedRead,CleanupExit::CheckedPanic) {
                return Err("unverified shared-read panic cleanup path");
            }
            Ok(catch_unwind(AssertUnwindSafe(|| {
                let _guard=SharedGuard {
                    lock:Some(self.value.read().unwrap_or_else(|err|err.into_inner())),
                    releases:&self.releases,
                };
                panic!("synthetic read callback panic");
            })))
        }
        pub fn release_count(&self)->usize {
            self.releases.load(Ordering::SeqCst)
        }
        pub fn try_write_available(&self)->bool {
            match self.value.try_write() {
                Ok(_guard)=>true,
                Err(TryLockError::Poisoned(error)) => {
                    drop(error.into_inner());
                    true
                }
                Err(TryLockError::WouldBlock)=>false,
            }
        }
    }
}

#[cfg(test)]
mod verifier_tests {
    use super::*;

    fn valid_paths() -> (Vec<CleanupPath>, Vec<(StateLeaseMode,bool)>) {
        let mode=StateLeaseMode::ExclusiveWrite;
        let make=|exit:CleanupExit| CleanupPath {
            lease:0,exit,pos:Pos{line:1,col:1},
            ops:vec![
                CleanupOp::Acquire{lease:0,mode:mode.clone()},
                CleanupOp::Release{lease:0},
                CleanupOp::Exit(exit),
            ],
        };
        (vec![make(CleanupExit::Normal),make(CleanupExit::CheckedPanic),
              make(CleanupExit::CooperativeCancel)],vec![(mode,true)])
    }

    #[test]
    fn cleanup_path_verifier_rejects_missing_double_and_wrong_unlocks() {
        let (good,types)=valid_paths();
        verify_paths(&good,&types).unwrap();
        let mut broken=good.clone();
        broken[0].ops.remove(1);
        assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
        let mut broken=good.clone();
        broken[0].ops.insert(2,CleanupOp::Release{lease:0});
        assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
        let mut broken=good.clone();
        broken[0].ops[1]=CleanupOp::Release{lease:1};
        assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
        let mut broken=good.clone();
        broken[0].ops[0]=CleanupOp::Acquire{
            lease:0,mode:StateLeaseMode::SharedRead
        };
        assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
    }

    #[test]
    fn cleanup_path_verifier_requires_all_relevant_exit_edges() {
        let (good,types)=valid_paths();
        for exit in [CleanupExit::Normal,CleanupExit::CheckedPanic,
            CleanupExit::CooperativeCancel] {
            let broken:Vec<_>=good.iter().filter(|p|p.exit!=exit).cloned().collect();
            assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
        }
        let mut broken=good.clone();
        broken.push(good[0].clone());
        assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
        let mut broken=good;
        broken[0].lease=5;
        assert_eq!(verify_paths(&broken,&types).unwrap_err().code,"E_CLEANUP");
    }
}
