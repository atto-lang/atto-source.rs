//! Trusted-host, nonblocking, scoped physical access to a guarded actor region.
//!
//! An Arc-backed coordinator binds the FIFO logical admission model to real
//! std::sync::RwLock protected memory. No guest code or raw pointers enter
//! this interface. It is a process-local reference, not a production actor
//! scheduler, actor identity authenticator, or hot-upgrade implementation.
use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError};

use crate::actor_state_guard_v1::{GuardError, GuardQueue, Mode, Permit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    Guard(GuardError),
    Busy,
    Poisoned,
    ForeignRegion,
    WrongMode,
}

impl From<GuardError> for RuntimeError {
    fn from(error: GuardError) -> Self {
        Self::Guard(error)
    }
}

/// Owns both the queue and the only physical state storage. Every memory
/// access occurs inside a non-escaping closure while an actual lock is held.
#[derive(Debug)]
pub struct GuardedActorRegion<T> {
    queue: Mutex<QueuedPermits>,
    // Scope Drop never waits for the queue mutex.
    releases: Sender<Permit>,
    state: RwLock<T>,
}

#[derive(Debug)]
struct QueuedPermits {
    guard: GuardQueue,
    releases: Receiver<Permit>,
}

impl QueuedPermits {
    // The physical lock has been dropped before a scope posts completion.
    // Draining under a short-lived queue lock linearizes completion against
    // retirement and admissions; a failure fails closed.
    fn drain_completed(&mut self) -> Result<(), RuntimeError> {
        while let Ok(permit) = self.releases.try_recv() {
            if self.guard.is_retired() {
                self.guard
                    .complete_physical_quiescence(&permit, permit.actor())?;
            } else {
                self.guard.release(&permit, permit.actor())?;
            }
        }
        Ok(())
    }
}

/// One logical admission. Not Clone and not Sync: safe Rust cannot
/// concurrently use the same actor's permit for overlapping callbacks.
#[must_use]
#[derive(Debug)]
pub struct ActorScope<T> {
    region: Arc<GuardedActorRegion<T>>,
    permit: Option<Permit>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<T> Drop for ActorScope<T> {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        // Never park a scheduler carrier while another thread holds the
        // queue mutex. The Arc retains the receiver until this scope drops.
        // If the channel were unexpectedly disconnected, the outstanding
        // guard stays charged rather than falsely proving quiescence.
        let _ = self.region.releases.send(permit);
    }
}

impl<T> GuardedActorRegion<T> {
    pub fn new(generation: u64, initial: T) -> Result<Arc<Self>, RuntimeError> {
        let (send, receive) = mpsc::channel();
        Ok(Arc::new(Self {
            queue: Mutex::new(QueuedPermits {
                guard: GuardQueue::new(generation)?,
                releases: receive,
            }),
            releases: send,
            state: RwLock::new(initial),
        }))
    }

    fn queue(&self) -> Result<MutexGuard<'_, QueuedPermits>, RuntimeError> {
        let mut lock = match self.queue.try_lock() {
            Ok(lock) => lock,
            Err(TryLockError::WouldBlock) => return Err(RuntimeError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(RuntimeError::Poisoned),
        };
        lock.drain_completed()?;
        Ok(lock)
    }

    /// Caller must already have independently authenticated an actor
    /// identity and its region capability. Numeric actor IDs aren't proof.
    pub fn request(
        &self,
        trusted_actor: u64,
        generation: u64,
        mode: Mode,
    ) -> Result<u64, RuntimeError> {
        Ok(self
            .queue()?
            .guard
            .request(trusted_actor, generation, mode)?)
    }

    /// No carrier sleep. A later reader cannot pass a queued writer.
    /// All returned scopes are owned by the trusted host, never the guest.
    pub fn admit_ready(self: &Arc<Self>) -> Result<Vec<ActorScope<T>>, RuntimeError> {
        let permits = self.queue()?.guard.admit_ready();
        Ok(permits
            .into_iter()
            .map(|permit| ActorScope {
                region: Arc::clone(self),
                permit: Some(permit),
                _not_sync: PhantomData,
            })
            .collect())
    }

    pub fn cancel_pending(
        &self,
        ticket: u64,
        trusted_actor: u64,
        generation: u64,
    ) -> Result<(), RuntimeError> {
        Ok(self
            .queue()?
            .guard
            .cancel_pending(ticket, trusted_actor, generation)?)
    }

    /// Logical STOP/upgrade fence. Already-entered closures may finish,
    /// but memory cannot be reclaimed until each physical scope exits.
    pub fn retire(&self, next_generation: u64) -> Result<(), RuntimeError> {
        Ok(self.queue()?.guard.retire(next_generation)?)
    }

    /// Necessary, not sufficient, for true heap reclamation. Callers must
    /// also own the allocation and ensure no FFI/raw-pointer escape exists.
    pub fn may_reclaim(&self) -> Result<bool, RuntimeError> {
        Ok(self.queue()?.guard.may_reclaim())
    }

    pub fn active_count(&self) -> Result<usize, RuntimeError> {
        Ok(self.queue()?.guard.active_count())
    }

    pub fn waiting_count(&self) -> Result<usize, RuntimeError> {
        Ok(self.queue()?.guard.waiting_count())
    }

    /// Consumes a read scope, preventing a loan from escaping via a guard
    /// handle. The compiler must reject await/select and reference escape
    /// inside the closure. A scope is released even during Rust unwinding.
    pub fn execute_read<R>(
        self: &Arc<Self>,
        scope: ActorScope<T>,
        callback: impl FnOnce(&T) -> R,
    ) -> Result<R, RuntimeError> {
        if !Arc::ptr_eq(self, &scope.region) {
            return Err(RuntimeError::ForeignRegion);
        }
        let state = match self.state.try_read() {
            Ok(value) => value,
            Err(TryLockError::WouldBlock) => return Err(RuntimeError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(RuntimeError::Poisoned),
        };
        {
            let queue = self.queue()?;
            let permit = scope.permit.as_ref().ok_or(RuntimeError::Poisoned)?;
            queue.guard.access(permit, permit.actor(), false)?;
        }
        let output = callback(&state);
        drop(state); // actual RwLock before logical loan completion
        drop(scope);
        Ok(output)
    }

    /// Physically exclusive access, guarded by an active writer permit.
    /// No mutable reference is returned to the caller.
    pub fn execute_write<R>(
        self: &Arc<Self>,
        scope: ActorScope<T>,
        callback: impl FnOnce(&mut T) -> R,
    ) -> Result<R, RuntimeError> {
        if !Arc::ptr_eq(self, &scope.region) {
            return Err(RuntimeError::ForeignRegion);
        }
        if scope.permit.as_ref().map(Permit::mode) != Some(Mode::Write) {
            return Err(RuntimeError::WrongMode);
        }
        let mut state = match self.state.try_write() {
            Ok(value) => value,
            Err(TryLockError::WouldBlock) => return Err(RuntimeError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(RuntimeError::Poisoned),
        };
        {
            let queue = self.queue()?;
            let permit = scope.permit.as_ref().ok_or(RuntimeError::Poisoned)?;
            queue.guard.access(permit, permit.actor(), true)?;
        }
        let output = callback(&mut state);
        drop(state);
        drop(scope);
        Ok(output)
    }
}

#[cfg(test)]
mod deferred_release_tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn drop_never_waits_for_concurrently_held_queue_mutex() {
        let region = GuardedActorRegion::new(1, 0_u64).unwrap();
        region.request(1, 1, Mode::Read).unwrap();
        let scope = region.admit_ready().unwrap().pop().unwrap();
        let held_queue = region.queue.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            drop(scope);
            tx.send(()).unwrap();
        });
        let result = rx.recv_timeout(Duration::from_secs(5));
        drop(held_queue);
        worker.join().unwrap();
        assert!(result.is_ok(), "scope Drop parked waiting for queue mutex");
        assert_eq!(region.active_count(), Ok(0));
    }

    #[test]
    fn deferred_scope_release_drains_after_stop_without_early_reclaim() {
        let region = GuardedActorRegion::new(4, 10_u64).unwrap();
        region.request(42, 4, Mode::Write).unwrap();
        let scope = region.admit_ready().unwrap().pop().unwrap();
        region.retire(5).unwrap();
        let held_queue = region.queue.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            drop(scope);
            tx.send(()).unwrap();
        });
        let result = rx.recv_timeout(Duration::from_secs(5));
        // No other method could observe completion while queue held.
        assert_eq!(held_queue.guard.active_count(), 1);
        drop(held_queue);
        worker.join().unwrap();
        assert!(result.is_ok(), "retired scope Drop parked");
        assert_eq!(region.may_reclaim(), Ok(true));
        assert_eq!(
            region.request(7, 5, Mode::Write),
            Err(RuntimeError::Guard(GuardError::Retired))
        );
    }
}
