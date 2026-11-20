//! Host-reference, single-arbiter, NON-GUEST mutex waiter state machine.
//!
//! This module deliberately does not implement the Atto Mutex<T> type,
//! a scheduler, tenant isolation or an async ABI. A privileged scheduler
//! would serialize calls and mint continuation identities. No blocking OS
//! primitive, thread spawn, spin-loop or guest pointer is used here.
//! Never treat these unit tests as compiled-guest / TenantVM security evidence.
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Domain {
    tenant: u64,
    actor: u64,
    incarnation: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Continuation {
    domain: Domain,
    logical_task: u64,
    segment: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket {
    generation: u64,
    sequence: u64,
    continuation: Continuation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Granted(Ticket),
    Waiting(Ticket),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Closed,
    ForeignDomain,
    Reentrant,
    QueueFull,
    Exhausted,
    StaleTicket,
    CancellationPending,
    AlreadyClaimed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cancellation {
    Queued,
    GrantedAndRevoked,
    /// Active lease cancellation is deferred until cleanup releases it.
    ActiveDeferred,
}

/// This intentionally exposes no constructor for an untrusted guest.
/// The caller must be an already privileged, serialized host scheduler.
pub(crate) struct WaitQueue {
    domain: Domain,
    generation: u64,
    next_sequence: u64,
    max_waiters: usize,
    closed: bool,
    owner: Option<Ticket>,
    active: bool,
    cancel_requested: bool,
    waiting: VecDeque<Ticket>,
    // At most one outstanding direct handoff wakeup for the current owner.
    wakeup: Option<Ticket>,
}
impl WaitQueue {
    pub(crate) fn new(domain: Domain, generation: u64, max_waiters: usize) -> Result<Self, Error> {
        if generation == 0 || max_waiters == 0 || max_waiters > 8192 {
            return Err(Error::Exhausted);
        }
        Ok(Self {
            domain, generation, next_sequence: 1, max_waiters, closed: false,
            owner: None, active: false, cancel_requested: false,
            waiting: VecDeque::new(), wakeup: None,
        })
    }

    fn check_domain(&self, who: Continuation) -> Result<(), Error> {
        if who.domain != self.domain { Err(Error::ForeignDomain) } else { Ok(()) }
    }
    fn fresh(&mut self, who: Continuation) -> Result<Ticket, Error> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(Ticket { generation: self.generation, sequence, continuation: who })
    }

    pub(crate) fn acquire(&mut self, who: Continuation) -> Result<Admission, Error> {
        if self.closed { return Err(Error::Closed); }
        self.check_domain(who)?;
        if self.owner.is_some_and(|t| t.continuation == who)
            || self.waiting.iter().any(|t| t.continuation == who) {
            return Err(Error::Reentrant);
        }
        // Admission and the capacity check happen before minting a ticket.
        // A failed request never silently consumes owner or queue rights.
        if self.owner.is_some() && self.waiting.len() >= self.max_waiters {
            return Err(Error::QueueFull);
        }
        let token = self.fresh(who)?;
        if self.owner.is_none() {
            debug_assert!(self.waiting.is_empty());
            self.owner = Some(token);
            Ok(Admission::Granted(token))
        } else {
            self.waiting.push_back(token);
            Ok(Admission::Waiting(token))
        }
    }

    fn handoff(&mut self) {
        // Only a privileged reactor may drain this wakeup; any token it
        // delivers must still be checked with `claim` on resume.
        let next = self.waiting.pop_front();
        self.owner = next;
        self.active = false;
        self.cancel_requested = false;
        self.wakeup = next;
    }

    pub(crate) fn release(&mut self, owner: Ticket) -> Result<(), Error> {
        if self.owner != Some(owner) || owner.generation != self.generation {
            return Err(Error::StaleTicket);
        }
        // An active task can still unwind and release after shutdown, but
        // shutdown must not hand this lock to another task.
        if self.closed {
            self.owner = None;
            self.active = false;
            self.cancel_requested = false;
            self.wakeup = None;
        } else {
            self.handoff();
        }
        Ok(())
    }

    /// A wakeup is a hint to the scheduler, not a capability that can bypass
    /// revalidation after cancellation/revocation. It is not exposed to guest.
    pub(crate) fn take_wakeup(&mut self) -> Option<Ticket> {
        self.wakeup.take()
    }

    pub(crate) fn claim(&mut self, token: Ticket) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        self.check_domain(token.continuation)?;
        if self.owner != Some(token) || token.generation != self.generation {
            return Err(Error::StaleTicket);
        }
        if self.cancel_requested { return Err(Error::CancellationPending); }
        if self.active { return Err(Error::AlreadyClaimed); }
        // Claim is the guarded-entry event. An active grant cannot be revoked.
        self.active = true;
        if self.wakeup == Some(token) { self.wakeup = None; }
        Ok(())
    }

    /// Metadata is not proof that a guest continuation physically retired.
    pub(crate) fn has_active_lease(&self) -> bool { self.owner.is_some() && self.active }
    pub(crate) fn cancellation_pending(&self) -> bool { self.cancel_requested }
    pub(crate) fn has_outstanding_grant(&self) -> bool { self.owner.is_some() }

    /// Same ticket cancels either a still-pending request or its already-
    /// granted handoff. The arbiter serializes cancellation versus release.
    pub(crate) fn cancel(&mut self, token: Ticket) -> Result<Cancellation, Error> {
        self.check_domain(token.continuation)?;
        if token.generation != self.generation { return Err(Error::StaleTicket); }
        if self.closed {
            // A shutdown may have closed new admission while an active holder
            // still needs a safe-point unwind; cancellation remains idempotent.
            if self.owner == Some(token) && self.active {
                self.cancel_requested = true;
                return Ok(Cancellation::ActiveDeferred);
            }
            return Err(Error::Closed);
        }
        if let Some(i) = self.waiting.iter().position(|t| *t == token) {
            self.waiting.remove(i);
            return Ok(Cancellation::Queued);
        }
        if self.owner == Some(token) {
            if self.active {
                // Request cancellation and wait for the holder to run its
                // lexical using/defer cleanup. No successor may be granted yet.
                self.cancel_requested = true;
                return Ok(Cancellation::ActiveDeferred);
            }
            // Pending handoff has not entered the critical section.
            self.handoff();
            return Ok(Cancellation::GrantedAndRevoked);
        }
        Err(Error::StaleTicket)
    }

    /// Shutdown cancels waiters and disallows new grants, but cannot revoke
    /// a task actively inside its critical section. Retain active owner until
    /// lexical cleanup calls release(). Task-scope quiescence is still needed.
    pub(crate) fn shutdown(&mut self) -> usize {
        self.closed = true;
        self.wakeup = None;
        let retired = self.waiting.len();
        self.waiting.clear();
        if self.active {
            // Inform active holder to unwind; do NOT publish a free lock.
            self.cancel_requested = true;
        } else {
            self.owner = None;
            self.cancel_requested = false;
        }
        retired
    }
}

/// Host-only, serialized task-local lexical guard stack. Caller MUST stop
/// executing protected guest code before invoking unwind_after_safe_point().
/// This is not an Atto scheduler and does not itself prove descendant quiescence.
pub(crate) struct TaskLeaseStack {
    continuation: Continuation,
    held: Vec<(u64, Ticket)>,
    cancellation_requested: bool,
}
impl TaskLeaseStack {
    pub(crate) fn new(continuation: Continuation) -> Self {
        Self { continuation, held: Vec::new(), cancellation_requested: false }
    }
    pub(crate) fn enter(
        &mut self, lock_id: u64, lock: &mut WaitQueue, ticket: Ticket,
    ) -> Result<(), Error> {
        if self.cancellation_requested { return Err(Error::CancellationPending); }
        if lock_id == 0 || ticket.continuation != self.continuation {
            return Err(Error::ForeignDomain);
        }
        if self.held.iter().any(|(id, _)| *id == lock_id) {
            return Err(Error::Reentrant);
        }
        if self.held.len() >= 256 { return Err(Error::Exhausted); }
        lock.claim(ticket)?;
        self.held.push((lock_id, ticket));
        Ok(())
    }
    /// Idempotent cancellation request. It never releases an active lock;
    /// each active holder must run lexical cleanup at a legal safe point.
    pub(crate) fn request_cancel(
        &mut self, locks: &mut BTreeMap<u64, WaitQueue>,
    ) -> Result<(), Error> {
        if self.cancellation_requested { return Ok(()); }
        // Validate the entire registered ledger before modifying any lock.
        for (id, ticket) in &self.held {
            let lock = locks.get(id).ok_or(Error::StaleTicket)?;
            if lock.owner != Some(*ticket) || !lock.active {
                return Err(Error::StaleTicket);
            }
        }
        for (id, ticket) in &self.held {
            let lock = locks.get_mut(id).ok_or(Error::StaleTicket)?;
            match lock.cancel(*ticket)? {
                Cancellation::ActiveDeferred => {}
                _ => return Err(Error::StaleTicket),
            }
        }
        self.cancellation_requested = true;
        Ok(())
    }
    /// Local cleanup of ALL successfully entered guards, in reverse
    /// acquisition order. A failed release is retained for quarantine/retry,
    /// while independent guards still get their own release attempt.
    /// This does NOT mark the TaskScope or its descendants physically retired.
    pub(crate) fn unwind_after_safe_point(
        &mut self, locks: &mut BTreeMap<u64, WaitQueue>,
    ) -> Result<Vec<u64>, Error> {
        let mut failed = Vec::new();
        let mut released = Vec::new();
        for &(id, ticket) in self.held.iter().rev() {
            let result = locks.get_mut(&id).ok_or(Error::StaleTicket)
                .and_then(|lock| lock.release(ticket));
            match result {
                Ok(()) => released.push(id),
                Err(_) => failed.push((id, ticket)),
            }
        }
        // Preserve acquisition order for future retry, not unwind order.
        failed.reverse();
        self.held = failed;
        if self.held.is_empty() { Ok(released) } else { Err(Error::StaleTicket) }
    }
    pub(crate) fn all_local_locks_released(&self) -> bool { self.held.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d() -> Domain { Domain { tenant: 1, actor: 9, incarnation: 2 } }
    fn c(task: u64) -> Continuation {
        Continuation { domain: d(), logical_task: task, segment: task }
    }
    fn queue(max: usize) -> WaitQueue { WaitQueue::new(d(), 7, max).unwrap() }
    fn owned(a: Admission) -> Ticket {
        match a { Admission::Granted(t) => t, Admission::Waiting(_) => panic!("not granted") }
    }
    fn pending(a: Admission) -> Ticket {
        match a { Admission::Waiting(t) => t, Admission::Granted(_) => panic!("not pending") }
    }

    #[test]
    fn immediate_grant_and_double_release_fails_closed() {
        let mut q = queue(2);
        let t = owned(q.acquire(c(1)).unwrap());
        q.claim(t).unwrap();
        q.release(t).unwrap();
        assert_eq!(q.release(t), Err(Error::StaleTicket));
        assert_eq!(q.take_wakeup(), None);
    }

    #[test]
    fn fifo_handoff_and_stale_wake_are_fenced() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        let b = pending(q.acquire(c(2)).unwrap());
        let c = pending(q.acquire(c(3)).unwrap());
        q.release(a).unwrap();
        assert_eq!(q.take_wakeup(), Some(b));
        q.claim(b).unwrap();
        q.release(b).unwrap();
        assert_eq!(q.take_wakeup(), Some(c));
        q.claim(c).unwrap();
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        q.release(c).unwrap();
        assert_eq!(q.take_wakeup(), None);
    }

    #[test]
    fn cancellation_before_and_after_handoff_never_double_grants() {
        let mut q = queue(4);
        let a = owned(q.acquire(c(1)).unwrap());
        let b = pending(q.acquire(c(2)).unwrap());
        let queued_c = pending(q.acquire(c(3)).unwrap());
        assert_eq!(q.cancel(b), Ok(Cancellation::Queued));
        q.release(a).unwrap();
        assert_eq!(q.take_wakeup(), Some(queued_c));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        assert_eq!(q.cancel(queued_c), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.claim(queued_c), Err(Error::StaleTicket));
        assert_eq!(q.release(queued_c), Err(Error::StaleTicket));
        assert!(matches!(q.acquire(c(4)), Ok(Admission::Granted(_))));
    }

    #[test]
    fn canceled_unconsumed_wakeup_is_not_delivered() {
        let mut q = queue(4);
        let a = owned(q.acquire(c(1)).unwrap());
        let b = pending(q.acquire(c(2)).unwrap());
        let c = pending(q.acquire(c(3)).unwrap());
        q.release(a).unwrap();
        assert_eq!(q.cancel(b), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), Some(c));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        q.claim(c).unwrap();
    }

    #[test]
    fn foreign_tenant_actor_and_generation_fail_closed() {
        let mut q = queue(2);
        let foreign = Continuation {
            domain: Domain { tenant: 2, ..d() },
            logical_task: 2, segment: 2,
        };
        assert_eq!(q.acquire(foreign), Err(Error::ForeignDomain));
        let t = owned(q.acquire(c(1)).unwrap());
        let old = Ticket { generation: 6, ..t };
        assert_eq!(q.claim(old), Err(Error::StaleTicket));
        assert_eq!(q.release(old), Err(Error::StaleTicket));
        assert_eq!(q.cancel(old), Err(Error::StaleTicket));
    }

    #[test]
    fn recursion_and_quota_are_refused_without_parking() {
        let mut q = queue(1);
        let a = owned(q.acquire(c(1)).unwrap());
        assert_eq!(q.acquire(c(1)), Err(Error::Reentrant));
        let b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.acquire(c(2)), Err(Error::Reentrant));
        assert_eq!(q.acquire(c(3)), Err(Error::QueueFull));
        q.release(a).unwrap();
        q.claim(b).unwrap();
    }

    #[test]
    fn shutdown_revokes_active_and_queued_ledger() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        let _b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.shutdown(), 1);
        assert_eq!(q.claim(a), Err(Error::Closed));
        assert_eq!(q.acquire(c(3)), Err(Error::Closed));
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.shutdown(), 0);
    }

    #[test]
    fn exhausted_epoch_never_reuses_a_ticket() {
        let mut q = queue(2);
        q.next_sequence = u64::MAX;
        assert_eq!(q.acquire(c(1)), Err(Error::Exhausted));
        assert_eq!(q.owner, None);
        assert!(q.waiting.is_empty());
    }

    #[test]
    fn cancelling_active_lease_waits_for_guard_unwind_before_handoff() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.cancel(a), Ok(Cancellation::ActiveDeferred));
        assert_eq!(q.cancel(a), Ok(Cancellation::ActiveDeferred));
        assert!(q.cancellation_pending() && q.has_active_lease());
        assert_eq!(q.take_wakeup(), None);
        assert_eq!(q.claim(a), Err(Error::CancellationPending));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        // Called by the task cleanup path after it stops using the lease.
        q.release(a).unwrap();
        assert!(!q.cancellation_pending());
        assert_eq!(q.take_wakeup(), Some(b));
        q.claim(b).unwrap();
        q.release(b).unwrap();
        assert!(!q.has_outstanding_grant());
    }
    #[test]
    fn unclaimed_handoff_may_be_revoked_at_cancellation() {
        let mut q = queue(3);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        let c = pending(q.acquire(c(3)).unwrap());
        q.release(a).unwrap();
        assert_eq!(q.cancel(b), Ok(Cancellation::GrantedAndRevoked));
        assert_eq!(q.take_wakeup(), Some(c));
        assert_eq!(q.claim(b), Err(Error::StaleTicket));
        q.claim(c).unwrap();
        q.release(c).unwrap();
    }
    #[test]
    fn shutdown_retains_active_owner_until_explicit_cleanup_acknowledgment() {
        let mut q = queue(2);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        let b = pending(q.acquire(c(2)).unwrap());
        assert_eq!(q.shutdown(), 1);
        assert!(q.has_outstanding_grant() && q.has_active_lease());
        assert_eq!(q.claim(b), Err(Error::Closed));
        assert_eq!(q.acquire(c(3)), Err(Error::Closed));
        assert_eq!(q.release(b), Err(Error::StaleTicket));
        q.release(a).unwrap();
        assert!(!q.has_outstanding_grant());
        assert_eq!(q.release(a), Err(Error::StaleTicket));
    }
    #[test]
    fn claimed_lease_cannot_replay_entry_or_reenter() {
        let mut q = queue(2);
        let a = owned(q.acquire(c(1)).unwrap());
        q.claim(a).unwrap();
        assert_eq!(q.claim(a), Err(Error::AlreadyClaimed));
        assert_eq!(q.acquire(c(1)), Err(Error::Reentrant));
        q.release(a).unwrap();
    }

    #[test]
    fn cancelled_task_unwinds_multiple_locks_lifo_before_successor_grants() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        let first = owned(locks.get_mut(&1).unwrap().acquire(c(1)).unwrap());
        scope.enter(1, locks.get_mut(&1).unwrap(), first).unwrap();
        let second = owned(locks.get_mut(&2).unwrap().acquire(c(1)).unwrap());
        scope.enter(2, locks.get_mut(&2).unwrap(), second).unwrap();
        let waiting_first = pending(locks.get_mut(&1).unwrap().acquire(c(2)).unwrap());
        let waiting_second = pending(locks.get_mut(&2).unwrap().acquire(c(2)).unwrap());
        scope.request_cancel(&mut locks).unwrap();
        scope.request_cancel(&mut locks).unwrap();
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), None);
        assert_eq!(locks.get_mut(&2).unwrap().take_wakeup(), None);
        assert!(!scope.all_local_locks_released());
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), vec![2, 1]);
        assert!(scope.all_local_locks_released());
        assert_eq!(locks.get_mut(&2).unwrap().take_wakeup(), Some(waiting_second));
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), Some(waiting_first));
        locks.get_mut(&2).unwrap().claim(waiting_second).unwrap();
        locks.get_mut(&1).unwrap().claim(waiting_first).unwrap();
        locks.get_mut(&2).unwrap().release(waiting_second).unwrap();
        locks.get_mut(&1).unwrap().release(waiting_first).unwrap();
    }
    #[test]
    fn cleanup_attempts_remaining_locks_if_one_lease_is_lost() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        for id in [1_u64, 2_u64] {
            let q = locks.get_mut(&id).unwrap();
            let t = owned(q.acquire(c(1)).unwrap());
            scope.enter(id, q, t).unwrap();
        }
        // Inject a malformed/unavailable cleanup dependency.
        locks.remove(&2);
        assert_eq!(scope.unwind_after_safe_point(&mut locks), Err(Error::StaleTicket));
        assert!(!scope.all_local_locks_released());
        assert!(!locks.get(&1).unwrap().has_outstanding_grant());
        // Local task may not advertise physical retirement on this failure.
    }
    #[test]
    fn cancellation_forbids_new_enter_and_shutdown_still_allows_unwind() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        let a = owned(locks.get_mut(&1).unwrap().acquire(c(1)).unwrap());
        scope.enter(1, locks.get_mut(&1).unwrap(), a).unwrap();
        scope.request_cancel(&mut locks).unwrap();
        locks.get_mut(&1).unwrap().shutdown();
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), vec![1]);
        assert!(scope.all_local_locks_released());
        let new = owned(queue(2).acquire(c(1)).unwrap());
        assert_eq!(scope.enter(2, locks.get_mut(&1).unwrap(), new),
            Err(Error::CancellationPending));
    }

    #[test]
    fn late_cancel_after_shutdown_preserves_active_lock_until_cleanup() {
        let mut q = queue(2);
        let t = owned(q.acquire(c(1)).unwrap());
        q.claim(t).unwrap();
        assert_eq!(q.shutdown(), 0);
        assert!(q.cancellation_pending());
        assert_eq!(q.cancel(t), Ok(Cancellation::ActiveDeferred));
        assert!(q.has_outstanding_grant());
        q.release(t).unwrap();
        assert!(!q.has_outstanding_grant());
        assert_eq!(q.cancel(t), Err(Error::Closed));
    }
    #[test]
    fn pending_waiter_cancellation_never_releases_an_active_task_guard() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(3));
        let mut active_scope = TaskLeaseStack::new(c(1));
        let owner = owned(locks.get_mut(&1).unwrap().acquire(c(1)).unwrap());
        active_scope.enter(1, locks.get_mut(&1).unwrap(), owner).unwrap();
        let waiting = pending(locks.get_mut(&1).unwrap().acquire(c(2)).unwrap());
        assert_eq!(locks.get_mut(&1).unwrap().cancel(waiting), Ok(Cancellation::Queued));
        assert!(locks.get(&1).unwrap().has_active_lease());
        assert!(locks.get(&1).unwrap().has_outstanding_grant());
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), None);
        // A pending waiter's cancellation must not unlock another owner's guard.
        active_scope.request_cancel(&mut locks).unwrap();
        assert!(locks.get(&1).unwrap().has_active_lease());
        assert_eq!(locks.get_mut(&1).unwrap().claim(waiting), Err(Error::StaleTicket));
        assert_eq!(active_scope.unwind_after_safe_point(&mut locks).unwrap(), vec![1]);
        assert_eq!(locks.get_mut(&1).unwrap().take_wakeup(), None);
        assert!(!locks.get(&1).unwrap().has_outstanding_grant());
    }

    #[test]
    fn task_guard_cleanup_on_normal_exit_is_lifo_and_idempotent() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        for id in [1_u64, 2_u64] {
            let q = locks.get_mut(&id).unwrap();
            let grant = owned(q.acquire(c(1)).unwrap());
            scope.enter(id, q, grant).unwrap();
        }
        // Scoped cleanup applies to successful return as well as cancellation.
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), vec![2, 1]);
        assert_eq!(scope.unwind_after_safe_point(&mut locks).unwrap(), Vec::<u64>::new());
        assert!(scope.all_local_locks_released());
        for id in [1_u64, 2_u64] {
            assert!(!locks.get(&id).unwrap().has_outstanding_grant());
        }
    }

    #[test]
    fn cancellation_validation_failure_keeps_existing_guard_fenced() {
        let mut locks = BTreeMap::new();
        locks.insert(1, queue(2));
        locks.insert(2, queue(2));
        let mut scope = TaskLeaseStack::new(c(1));
        for id in [1_u64, 2_u64] {
            let q = locks.get_mut(&id).unwrap();
            let grant = owned(q.acquire(c(1)).unwrap());
            scope.enter(id, q, grant).unwrap();
        }
        locks.remove(&2); // inject loss of a required cleanup dependency
        assert_eq!(scope.request_cancel(&mut locks), Err(Error::StaleTicket));
        assert!(locks.get(&1).unwrap().has_active_lease());
        assert!(!locks.get(&1).unwrap().cancellation_pending());
        assert!(!scope.all_local_locks_released());
        // This mock is NOT permission to retire the Task or unlock a live guard.
        assert_eq!(scope.unwind_after_safe_point(&mut locks), Err(Error::StaleTicket));
        assert!(!locks.get(&1).unwrap().has_outstanding_grant());
        assert!(!scope.all_local_locks_released());
    }

}
