//! Trusted-host, nonblocking, scoped physical access to a guarded actor region.
//!
//! An Arc-backed coordinator binds the FIFO logical admission model to real
//! std::sync::RwLock protected memory. No guest code or raw pointers enter
//! this interface. It is a process-local reference, not a production actor
//! scheduler, actor identity authenticator, or hot-upgrade implementation.
use std::cell::Cell;
use std::marker::PhantomData;
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
    queue: Mutex<GuardQueue>,
    state: RwLock<T>,
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
        // Drop occurs after any physical RwLock guard in execute_read/write
        // has been released. A poisoned queue is never treated as quiescent.
        if let Ok(mut queue) = self.region.queue.lock() {
            if queue.is_retired() {
                let _ = queue.complete_physical_quiescence(&permit, permit.actor());
            } else {
                let _ = queue.release(&permit, permit.actor());
            }
        }
    }
}

impl<T> GuardedActorRegion<T> {
    pub fn new(generation: u64, initial: T) -> Result<Arc<Self>, RuntimeError> {
        Ok(Arc::new(Self {
            queue: Mutex::new(GuardQueue::new(generation)?),
            state: RwLock::new(initial),
        }))
    }

    fn queue(&self) -> Result<MutexGuard<'_, GuardQueue>, RuntimeError> {
        match self.queue.try_lock() {
            Ok(lock) => Ok(lock),
            Err(TryLockError::WouldBlock) => Err(RuntimeError::Busy),
            Err(TryLockError::Poisoned(_)) => Err(RuntimeError::Poisoned),
        }
    }

    /// Caller must already have independently authenticated an actor
    /// identity and its region capability. Numeric actor IDs aren't proof.
    pub fn request(
        &self,
        trusted_actor: u64,
        generation: u64,
        mode: Mode,
    ) -> Result<u64, RuntimeError> {
        Ok(self.queue()?.request(trusted_actor, generation, mode)?)
    }

    /// No carrier sleep. A later reader cannot pass a queued writer.
    /// All returned scopes are owned by the trusted host, never the guest.
    pub fn admit_ready(self: &Arc<Self>) -> Result<Vec<ActorScope<T>>, RuntimeError> {
        let permits = self.queue()?.admit_ready();
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
            .cancel_pending(ticket, trusted_actor, generation)?)
    }

    /// Logical STOP/upgrade fence. Already-entered closures may finish,
    /// but memory cannot be reclaimed until each physical scope exits.
    pub fn retire(&self, next_generation: u64) -> Result<(), RuntimeError> {
        Ok(self.queue()?.retire(next_generation)?)
    }

    /// Necessary, not sufficient, for true heap reclamation. Callers must
    /// also own the allocation and ensure no FFI/raw-pointer escape exists.
    pub fn may_reclaim(&self) -> Result<bool, RuntimeError> {
        Ok(self.queue()?.may_reclaim())
    }

    pub fn active_count(&self) -> Result<usize, RuntimeError> {
        Ok(self.queue()?.active_count())
    }

    pub fn waiting_count(&self) -> Result<usize, RuntimeError> {
        Ok(self.queue()?.waiting_count())
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
            queue.access(permit, permit.actor(), false)?;
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
            queue.access(permit, permit.actor(), true)?;
        }
        let output = callback(&mut state);
        drop(state);
        drop(scope);
        Ok(output)
    }
}
