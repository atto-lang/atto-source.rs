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

/// This type is intentionally non-Copy and does not implement Clone.
/// A ready lease has unique protected mutation authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AffineLeaseInt {
    state: LeaseState,
}
#[derive(Debug, PartialEq, Eq)]
enum LeaseState { Active, Released }
impl AffineLeaseInt {
    fn acquire_ready() -> Self { Self { state: LeaseState::Active } }

    /// This proof consumes a live lease, preventing second release in HIR.
    fn release(mut self) -> Result<Self, &'static str> {
        if self.state != LeaseState::Active { return Err("E_GUARD_DOUBLE_RELEASE"); }
        self.state = LeaseState::Released;
        Ok(self)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CheckedReadyLease {
    /// Protected owner value before any guest execution.
    initial: i64,
    /// Signed scalar update under the affine exclusive lease.
    delta: i64,
    /// No parallel or nested acquisition is admitted in this source profile.
    acquisitions: u8,
    /// Predicted copy-return results; these are proofs, not guest substitutes.
    expected: Vec<i64>,
}

fn pos() -> Pos { Pos { line: 1, col: 1 } }

impl CheckedReadyLease {
    pub(crate) fn prove(initial: i64, delta: i64, acquisitions: u8) -> Result<Self, Diagnostic> {
        if !(1..=2).contains(&acquisitions) {
            return Err(Diagnostic::new(pos(), "E_LOCK_UNSUPPORTED",
                "ready lease proof requires one or two sequential acquisitions"));
        }
        let mut expected = Vec::with_capacity(acquisitions as usize);
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
            expected.push(value);
        }
        Ok(Self { initial, delta, acquisitions, expected })
    }

    pub(crate) fn final_expected(&self) -> i64 {
        self.expected[self.acquisitions as usize - 1]
    }

    pub(crate) fn initial(&self) -> i64 { self.initial }

    fn int(n: i64) -> TE { TE { ty: Type::Int, e: TExpr::I64(n) } }
    fn global(index: u32) -> TE { TE { ty: Type::Int, e: TExpr::GlobalGet(index) } }

    /// Emit only a value-copy return after exactly-once lease release.
    /// This is a verified scalar-only HIR pattern: owner and occupancy
    /// globals remain module-private (Wasm exports only main/panic_code).
    pub(crate) fn lower_one_guard(&self) -> Vec<TStmt> {
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
        assert_eq!(one.final_expected(), 1);
        assert_eq!(one.initial(), 0);
        let two = CheckedReadyLease::prove(0, 1, 2).unwrap();
        assert_eq!(two.final_expected(), 2);
        assert_eq!(two.lower_one_guard().len(), 6);
        assert!(matches!(two.lower_one_guard()[4], TStmt::GlobalSet(1, _)));
        assert!(matches!(two.lower_one_guard()[5], TStmt::Return(_)));
    }

    #[test]
    fn resource_is_affine_and_released_once() {
        let lease = AffineLeaseInt::acquire_ready();
        assert_eq!(lease.state, LeaseState::Active);
        let done = lease.release().unwrap();
        assert_eq!(done.state, LeaseState::Released);
        assert_eq!(done.release().unwrap_err(), "E_GUARD_DOUBLE_RELEASE");
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
        assert_eq!(CheckedReadyLease::prove(17, 23, 1).unwrap().final_expected(), 40);
        assert_eq!(CheckedReadyLease::prove(17, -20, 2).unwrap().final_expected(), -23);
    }
}
