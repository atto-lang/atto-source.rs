//! Host-reference, single-arbiter, NON-GUEST mutex waiter state machine.
//!
//! This module deliberately does not implement the Atto Mutex<T> type,
//! a scheduler, tenant isolation or an async ABI. A privileged scheduler
//! would serialize calls and mint continuation identities. No blocking OS
//! primitive, thread spawn, spin-loop or guest pointer is used here.
//! Never treat these unit tests as compiled-guest / TenantVM security evidence.
use std::collections::VecDeque;

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
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cancellation {
    Queued,
    GrantedAndRevoked,
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
            owner: None, waiting: VecDeque::new(), wakeup: None,
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
        self.wakeup = next;
    }

    pub(crate) fn release(&mut self, owner: Ticket) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        if self.owner != Some(owner) || owner.generation != self.generation {
            return Err(Error::StaleTicket);
        }
        self.handoff();
        Ok(())
    }

    /// A wakeup is a hint to the scheduler, not a capability that can bypass
    /// revalidation after cancellation/revocation. It is not exposed to guest.
    pub(crate) fn take_wakeup(&mut self) -> Option<Ticket> {
        self.wakeup.take()
    }

    pub(crate) fn claim(&self, token: Ticket) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        self.check_domain(token.continuation)?;
        if self.owner != Some(token) || token.generation != self.generation {
            return Err(Error::StaleTicket);
        }
        Ok(())
    }

    /// Same ticket cancels either a still-pending request or its already-
    /// granted handoff. The arbiter serializes cancellation versus release.
    pub(crate) fn cancel(&mut self, token: Ticket) -> Result<Cancellation, Error> {
        if self.closed { return Err(Error::Closed); }
        self.check_domain(token.continuation)?;
        if token.generation != self.generation { return Err(Error::StaleTicket); }
        if let Some(i) = self.waiting.iter().position(|t| *t == token) {
            self.waiting.remove(i);
            return Ok(Cancellation::Queued);
        }
        if self.owner == Some(token) {
            self.handoff();
            return Ok(Cancellation::GrantedAndRevoked);
        }
        Err(Error::StaleTicket)
    }

    /// Privileged supervisor quiescence: active grants and pending waiters
    /// are invalidated, but physical guest heap reclamation is NOT proved.
    pub(crate) fn shutdown(&mut self) -> usize {
        self.closed = true;
        self.owner = None;
        self.wakeup = None;
        let retired = self.waiting.len();
        self.waiting.clear();
        retired
    }
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
}
