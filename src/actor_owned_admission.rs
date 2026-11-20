//! Host-only, typed and affine actor mailbox admission reference.
//!
//! NOT an Atto guest ABI, not a secure TenantVM broker. A production scheduler
//! must authenticate callers and capabilities independently, replace the
//! trusted booleans, and prove memory/queue reclamation. No cross-tenant alias
//! or unsafe pointer transfer is permitted. Payloads are moved, never cloned.
#![forbid(unsafe_code)]
#![cfg_attr(not(test), allow(dead_code))]
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActorIdentity {
    pub tenant: u64,
    pub actor: u64,
    pub incarnation: u64,
    pub placement: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionError {
    Unauthorized,
    Stale,
    Closed,
    Full,
    InvalidTicket,
    TicketExhausted,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    serial: u64,
    actor: ActorIdentity,
    epoch: u64,
    sender_tenant: u64,
}

struct Escrow<T> {
    payload: T,
    sender_tenant: u64,
    actor: ActorIdentity,
    epoch: u64,
}
pub(crate) struct Recovered<T> {
    pub sender_tenant: u64,
    pub payload: T,
}
pub(crate) struct Abandoned<T> {
    pub payload: T,
    pub actor: ActorIdentity,
}
#[must_use = "uncommitted messages must be returned to senders; abandoned mail must be retired"]
pub(crate) struct StopOutcome<T> {
    pub recovered: Vec<Recovered<T>>,
    pub abandoned: Vec<Abandoned<T>>,
}
pub(crate) struct Admission<T> {
    destination: ActorIdentity,
    epoch: u64,
    next_serial: u64,
    capacity: usize,
    closed: bool,
    staged: HashMap<u64, Escrow<T>>,
    queue: VecDeque<(u64, T)>,
    pub delivered: usize,
    pub abandoned: usize,
}

impl<T> Admission<T> {
    pub(crate) fn new(destination: ActorIdentity, epoch: u64, capacity: usize) -> Self {
        Self { destination, epoch, next_serial: 1, capacity, closed: false,
            staged: HashMap::new(), queue: VecDeque::new(), delivered: 0, abandoned: 0 }
    }

    // The bool MUST come from a trusted host grant, never a guest-supplied bit.
    // Every failure returns the original, uniquely owned T to its caller.
    pub(crate) fn stage(&mut self, sender_tenant: u64, actor: ActorIdentity,
                        epoch: u64, trusted_grant: bool, payload: T)
        -> Result<Ticket, (AdmissionError, T)> {
        if !trusted_grant || sender_tenant != self.destination.tenant {
            return Err((AdmissionError::Unauthorized, payload));
        }
        if actor != self.destination || epoch != self.epoch {
            return Err((AdmissionError::Stale, payload));
        }
        if self.closed { return Err((AdmissionError::Closed, payload)); }
        let Some(used) = self.staged.len().checked_add(self.queue.len()) else {
            return Err((AdmissionError::Full, payload));
        };
        if used >= self.capacity { return Err((AdmissionError::Full, payload)); }
        let Some(next) = self.next_serial.checked_add(1) else {
            return Err((AdmissionError::TicketExhausted, payload));
        };
        let serial = self.next_serial;
        self.next_serial = next;
        self.staged.insert(serial, Escrow { payload, sender_tenant, actor, epoch });
        Ok(Ticket { serial, actor, epoch, sender_tenant })
    }

    fn checked(&self, ticket: &Ticket) -> bool {
        self.staged.get(&ticket.serial).is_some_and(|escrow|
            escrow.sender_tenant == ticket.sender_tenant &&
            escrow.actor == ticket.actor &&
            escrow.epoch == ticket.epoch)
    }

    // The commit is the mailbox admission linearization point. If authorization
    // is revoked or generation changes, exactly one T is returned.
    // A forged/stale ticket never touches a different owner's escrow.
    pub(crate) fn commit(&mut self, ticket: Ticket, trusted_grant: bool)
        -> Result<u64, CommitError<T>> {
        if !self.checked(&ticket) {
            return Err(CommitError::InvalidTicket(AdmissionError::InvalidTicket));
        }
        let escrow = self.staged.remove(&ticket.serial).expect("checked");
        if !trusted_grant {
            return Err(CommitError::Recovered(AdmissionError::Unauthorized,
                Recovered { sender_tenant: escrow.sender_tenant, payload: escrow.payload }));
        }
        if self.destination != ticket.actor || self.epoch != ticket.epoch {
            return Err(CommitError::Recovered(AdmissionError::Stale,
                Recovered { sender_tenant: escrow.sender_tenant, payload: escrow.payload }));
        }
        if self.closed {
            return Err(CommitError::Recovered(AdmissionError::Closed,
                Recovered { sender_tenant: escrow.sender_tenant, payload: escrow.payload }));
        }
        self.queue.push_back((ticket.serial, escrow.payload));
        Ok(ticket.serial)
    }

    pub(crate) fn abort(&mut self, ticket: Ticket) -> Result<Recovered<T>, AdmissionError> {
        if !self.checked(&ticket) { return Err(AdmissionError::InvalidTicket); }
        let escrow = self.staged.remove(&ticket.serial).expect("checked");
        Ok(Recovered { sender_tenant: escrow.sender_tenant, payload: escrow.payload })
    }

    // "Delivered" only means handed to the caller, NOT handler completion.
    pub(crate) fn deliver(&mut self) -> Option<(u64, T)> {
        let message = self.queue.pop_front();
        if message.is_some() { self.delivered += 1; }
        message
    }

    pub(crate) fn stop(&mut self) -> StopOutcome<T> {
        self.closed = true;
        let recovered = self.staged.drain().map(|(_, escrow)| Recovered {
            sender_tenant: escrow.sender_tenant, payload: escrow.payload
        }).collect();
        let abandoned: Vec<_> = self.queue.drain(..).map(|(_, payload)| Abandoned {
            payload, actor: self.destination
        }).collect();
        self.abandoned += abandoned.len();
        StopOutcome { recovered, abandoned }
    }

    pub(crate) fn restart(&mut self, actor: ActorIdentity, epoch: u64) -> StopOutcome<T> {
        let outcome = self.stop();
        self.destination = actor;
        self.epoch = epoch;
        self.closed = false;
        outcome
    }
}

pub(crate) enum CommitError<T> {
    InvalidTicket(AdmissionError),
    Recovered(AdmissionError, Recovered<T>),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn actor() -> ActorIdentity {
        ActorIdentity { tenant: 7, actor: 42, incarnation: 3, placement: 2 }
    }
    #[test] fn exact_one_owner_on_success() {
        let mut m = Admission::new(actor(), 1, 1);
        let t = m.stage(7, actor(), 1, true, String::from("parcel")).unwrap();
        assert_eq!(m.commit(t, true).unwrap(), 1);
        assert_eq!(m.deliver().unwrap().1, "parcel");
        assert_eq!(m.delivered, 1);
        assert_eq!(m.abandoned, 0);
    }
    #[test] fn full_mailbox_returns_original_owned_payload() {
        let mut m = Admission::new(actor(), 1, 1);
        let t = m.stage(7, actor(), 1, true, Box::new(17)).unwrap();
        let (_, value) = m.stage(7, actor(), 1, true, Box::new(42)).unwrap_err();
        assert_eq!(*value, 42);
        assert_eq!(*m.abort(t).unwrap().payload, 17);
        assert!(m.deliver().is_none());
    }
    #[test] fn denied_and_cross_tenant_stage_return_owner() {
        let mut m = Admission::new(actor(), 1, 1);
        for (sender, grant) in [(7, false), (8, true)] {
            let (err, value) = m.stage(sender, actor(), 1, grant, vec![1, 2, 3]).unwrap_err();
            assert_eq!(err, AdmissionError::Unauthorized);
            assert_eq!(value, vec![1, 2, 3]);
        }
    }
    #[test] fn revoked_between_stage_and_commit_refunds_once() {
        let mut m = Admission::new(actor(), 1, 1);
        let ticket = m.stage(7, actor(), 1, true, vec![3, 4]).unwrap();
        let error = m.commit(ticket, false).err().unwrap();
        match error {
            CommitError::Recovered(AdmissionError::Unauthorized, payload) =>
                assert_eq!(payload.payload, vec![3, 4]),
            _ => panic!("expected exactly one recovered owner"),
        }
        assert!(m.deliver().is_none());
    }
    #[test] fn abort_before_commit_restores_owner() {
        let mut m = Admission::new(actor(), 1, 1);
        let ticket = m.stage(7, actor(), 1, true, 99).unwrap();
        assert_eq!(m.abort(ticket).unwrap().payload, 99);
        assert!(m.deliver().is_none());
    }
    #[test] fn stage_capacity_counts_inbox_and_escrow() {
        let mut m = Admission::new(actor(), 1, 2);
        let a = m.stage(7, actor(), 1, true, 1).unwrap();
        let b = m.stage(7, actor(), 1, true, 2).unwrap();
        assert_eq!(m.stage(7, actor(), 1, true, 3).unwrap_err().0, AdmissionError::Full);
        assert_eq!(m.commit(a, true), Ok(1));
        assert_eq!(m.stage(7, actor(), 1, true, 4).unwrap_err().0, AdmissionError::Full);
        assert_eq!(m.abort(b).unwrap().payload, 2);
        let c = m.stage(7, actor(), 1, true, 3).unwrap();
        assert_eq!(m.commit(c, true), Ok(3));
    }
    #[test] fn close_returns_escrow_and_abandons_only_admitted() {
        let mut m = Admission::new(actor(), 1, 3);
        let a = m.stage(7, actor(), 1, true, 17).unwrap();
        let b = m.stage(7, actor(), 1, true, 25).unwrap();
        m.commit(a, true).unwrap();
        let outcome = m.stop();
        assert_eq!(outcome.recovered.len(), 1);
        assert_eq!(outcome.recovered[0].payload, 25);
        assert_eq!(outcome.abandoned.len(), 1);
        assert_eq!(outcome.abandoned[0].payload, 17);
        assert_eq!(m.abandoned, 1);
        assert_eq!(m.abort(b), Err(AdmissionError::InvalidTicket));
        let second = m.stop();
        assert_eq!(second.recovered.len(), 0);
        assert_eq!(second.abandoned.len(), 0);
        assert_eq!(m.abandoned, 1);
    }
    #[test] fn restart_fences_old_incarnation_and_epoch() {
        let mut m = Admission::new(actor(), 1, 2);
        let a = m.stage(7, actor(), 1, true, 17).unwrap();
        m.commit(a, true).unwrap();
        let mut next = actor();
        next.incarnation += 1;
        let result = m.restart(next, 2);
        assert_eq!(result.abandoned.len(), 1);
        assert_eq!(m.abandoned, 1);
        assert_eq!(m.stage(7, actor(), 1, true, 1).unwrap_err().0, AdmissionError::Stale);
        assert_eq!(m.stage(7, next, 2, true, 2).unwrap().serial, 2);
    }
    #[test] fn forged_ticket_cannot_take_another_senders_payload() {
        let mut m = Admission::new(actor(), 1, 1);
        let actual = m.stage(7, actor(), 1, true, "value").unwrap();
        let mut forged = Ticket { serial: actual.serial, actor: actual.actor,
            epoch: actual.epoch, sender_tenant: 8 };
        assert!(matches!(m.commit(forged, true), Err(CommitError::InvalidTicket(_))));
        forged = Ticket { serial: actual.serial, actor: actual.actor,
            epoch: 999, sender_tenant: 7 };
        assert!(matches!(m.commit(forged, true), Err(CommitError::InvalidTicket(_))));
        assert_eq!(m.abort(actual).unwrap().payload, "value");
    }
    #[test] fn pre-admission_failure_does_not_drop_owned_payload() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        struct Tracked { drops: Arc<AtomicUsize> }
        impl Drop for Tracked {
            fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let mut m = Admission::new(actor(), 1, 0);
        let (err, recovered) = m.stage(7, actor(), 1, true,
            Tracked { drops: drops.clone() }).err().unwrap();
        assert_eq!(err, AdmissionError::Full);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(recovered);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
