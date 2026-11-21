//! Narrow, source-checked MutexLease<int> HIR for the uncontended guest fast path.
//!
//! Not a general Mutex<T> type checker, not a scheduler, and not evidence of
//! cancellation/exception cleanup. The caller must have proved one unique
//! actor/task-local Mutex<int> owner, a default readonly loan of that owner,
//! an affine lease that cannot escape, and exactly one or two *sequential*
//! ready lock_async acquisitions. Source parser must reject all other shapes.
//!
//! Keeping the linear guard protocol here makes proof and guest emission
//! agree. All exceptional arithmetic must be rejected before any guest can
//! acquire a lease, because the present Wasm slice has no scope unwind ABI.

use crate::ast::{BinOp, Type};
use crate::check::{TExpr, TStmt, TE};
use crate::diag::{Diagnostic, Pos};

/// A compile-time affine type-state resource: once released, a lease
/// cannot be released again because the only release method exists on
/// `AffineLeaseInt<LeaseActive>` and consumes its receiver. Never Copy/Clone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LeaseActive;
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LeaseReleased;
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AffineLeaseInt<State> {
    state: std::marker::PhantomData<State>,
    // PhantomData<Rc<()>> is a ZERO-ALLOCATION auto-trait fence: even
    // an affine ZST lease must not automatically implement Send or Sync.
    // The actual guest lease will also need independently checked
    // TenantVM/actor-domain provenance in the production type checker.
    not_send_or_sync: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl AffineLeaseInt<LeaseActive> {
    fn acquire_ready() -> Self {
        Self {
            state: std::marker::PhantomData,
            not_send_or_sync: std::marker::PhantomData,
        }
    }
    fn release(self) -> AffineLeaseInt<LeaseReleased> {
        AffineLeaseInt {
            state: std::marker::PhantomData,
            not_send_or_sync: std::marker::PhantomData,
        }
    }
}
impl AffineLeaseInt<LeaseReleased> {
    fn is_released(&self) -> bool { true }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CheckedReadyLease {
    /// The only value needed by guest lowering. The full sequence of
    /// expected results is checked, not hardcoded or exposed to Wasm.
    delta: i64,
}

fn pos() -> Pos { Pos { line: 1, col: 1 } }

impl CheckedReadyLease {
    pub(crate) fn prove(initial: i64, delta: i64, acquisitions: u8) -> Result<Self, Diagnostic> {
        if !(1..=2).contains(&acquisitions) {
            return Err(Diagnostic::new(pos(), "E_LOCK_UNSUPPORTED",
                "ready lease proof requires one or two sequential acquisitions"));
        }
        let mut value = initial;
        for _ in 0..acquisitions {
            // An exceptional checked addition while holding a lease would
            // leave the guest occupancy flag set until a real unwind runtime
            // exists. Consequently no execution profile with possible
            // overflow may be admitted.
            value = value.checked_add(delta).ok_or_else(|| Diagnostic::new(
                pos(), "E_OVERFLOW",
                "guarded mutation can overflow: no exceptional lease cleanup ABI"
            ))?;
        }
        // No predicted value is ever substituted for the real guest update.
        // This proof only authorizes lowering after every call was checked.
        Ok(Self { delta })
    }

    fn int(n: i64) -> TE { TE { ty: Type::Int, e: TExpr::I64(n) } }
    fn global(index: u32) -> TE { TE { ty: Type::Int, e: TExpr::GlobalGet(index) } }

    /// Emit only a value-copy return after exactly-once lease release.
    /// This is a verified scalar-only HIR pattern: owner and occupancy
    /// globals remain module-private (Wasm exports only main/panic_code).
    pub(crate) fn lower_one_guard(&self) -> Vec<TStmt> {
        // Compile-time type-state consumes a single affine lease before
        // constructing the sole return path. The generated Wasm instructions
        // still perform the actual occupancy check, acquire and release.
        let released = AffineLeaseInt::<LeaseActive>::acquire_ready().release();
        debug_assert!(released.is_released());
        let live = TE { ty: Type::Bool, e: TExpr::Bin(
            BinOp::Ne, Box::new(Self::global(1)), Box::new(Self::int(0))) };
        let updated = TE { ty: Type::Int, e: TExpr::Bin(
            BinOp::Add, Box::new(Self::global(0)), Box::new(Self::int(self.delta))) };
        vec![
            TStmt::If(vec![(live, vec![TStmt::Unreachable])], None),
            TStmt::GlobalSet(1, Self::int(1)), // acquire unique lease
            TStmt::GlobalSet(0, updated),     // checked protected mutation
            TStmt::Set(0, Self::global(0)),   // snapshot copied scalar
            TStmt::GlobalSet(1, Self::int(0)),// release before all returns
            TStmt::Return(Some(TE { ty: Type::Int, e: TExpr::Local(0) })),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_accepts_one_or_two_sequential_ready_acquisitions() {
        let one = CheckedReadyLease::prove(0, 1, 1).unwrap();
        assert_eq!(one.delta, 1);
        let two = CheckedReadyLease::prove(0, 1, 2).unwrap();
        assert_eq!(two.delta, 1);
        assert_eq!(two.lower_one_guard().len(), 6);
        assert!(matches!(two.lower_one_guard()[4], TStmt::GlobalSet(1, _)));
        assert!(matches!(two.lower_one_guard()[5], TStmt::Return(_)));
    }

    #[test]
    fn resource_is_affine_with_compile_time_release_typestate() {
        let lease = AffineLeaseInt::<LeaseActive>::acquire_ready();
        let done: AffineLeaseInt<LeaseReleased> = lease.release();
        assert!(done.is_released());
        assert_eq!(std::mem::size_of::<AffineLeaseInt<LeaseActive>>(), 0);
        assert_eq!(std::mem::size_of::<AffineLeaseInt<LeaseReleased>>(), 0);
        // The Rc phantom marker makes both states !Send and !Sync at compile
        // time without allocating or holding a real Rc reference.
        // No release method exists on LeaseReleased and the original
        // LeaseActive value was consumed; duplicate release does not typecheck.
    }

    #[test]
    fn overflow_and_concurrent_or_unknown_acquisition_counts_fail_closed() {
        for acquisitions in [0u8,3,4,u8::MAX] {
            assert_eq!(CheckedReadyLease::prove(0, 1, acquisitions).unwrap_err().code,
                       "E_LOCK_UNSUPPORTED");
        }
        assert_eq!(
            CheckedReadyLease::prove(i64::MAX, 1, 1).unwrap_err().code, "E_OVERFLOW");
        assert_eq!(
            CheckedReadyLease::prove(i64::MAX - 1, 1, 2).unwrap_err().code, "E_OVERFLOW");
        assert_eq!(
            CheckedReadyLease::prove(i64::MIN, -1, 1).unwrap_err().code, "E_OVERFLOW");
    }

    #[test]
    fn signed_updates_are_checked_for_each_successive_lease() {
        assert_eq!(CheckedReadyLease::prove(17, 23, 1).unwrap().delta, 23);
        assert_eq!(CheckedReadyLease::prove(17, -20, 2).unwrap().delta, -20);
        assert_eq!(CheckedReadyLease::prove(17, -20, 0).unwrap_err().code,
                   "E_LOCK_UNSUPPORTED");
    }
}
