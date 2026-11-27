//! Owner-thread-only host stdio admission substrate. No guest ABI, sink thread,
//! or scheduler authority: the trusted owner must validate the live execution
//! generation and deliver readiness only on that execution's scheduler.
#![forbid(unsafe_code)]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_DESCRIPTOR: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Owner {
    pub execution: u64,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ticket {
    descriptor: u64,
    id: u64,
    owner: Owner,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkTicket { descriptor: u64, id: u64 }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidOwner, Oversize, Quota, Closed, SinkFailed, Exhausted, Stale,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Accepted,
    Pending(Ticket),
}
struct Record {
    id: u64,
    text: String,
}
struct Pending {
    ticket: Ticket,
    text: String,
}
struct Ready {
    ticket: Ticket,
    result: Result<(), Error>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Accounting {
    pub accepted_records: usize,
    pub accepted_bytes: usize,
    pub pending_records: usize,
    pub pending_bytes: usize,
    pub retained_receipts: usize,
}
/// One descriptor. Caller must serialize access on its runtime owner lane.
pub struct BufferedStdio {
    records: VecDeque<Record>,
    pending: VecDeque<Pending>,
    ready: VecDeque<Ready>,
    active: Option<(u64, usize)>,
    accepted_bytes: usize,
    pending_bytes: usize,
    max_records: usize,
    max_bytes: usize,
    max_waiters: usize,
    max_pending: usize,
    next: u64,
    descriptor: u64,
    sealed: bool,
    failure: Option<Error>,
}
impl BufferedStdio {
    pub fn new(max_records: usize, max_bytes: usize, max_waiters: usize, max_pending: usize) -> Result<Self, Error> {
        if !(1..=1024).contains(&max_records) || !(1..=4_194_304).contains(&max_bytes)
            || !(1..=64).contains(&max_waiters) || !(1..=1_048_576).contains(&max_pending) {
            return Err(Error::Quota);
        }
        let mut last = NEXT_DESCRIPTOR.load(Ordering::Relaxed);
        let descriptor = loop {
            let next = last.checked_add(1).ok_or(Error::Exhausted)?;
            match NEXT_DESCRIPTOR.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break next,
                Err(observed) => last = observed,
            }
        };
        Ok(Self {
            records: VecDeque::new(), pending: VecDeque::new(), ready: VecDeque::new(),
            active: None, accepted_bytes: 0, pending_bytes: 0,
            max_records, max_bytes, max_waiters, max_pending,
            next: 0, descriptor, sealed: false, failure: None,
        })
    }
    pub fn accounting(&self) -> Accounting {
        Accounting {
            accepted_records: self.records.len() + usize::from(self.active.is_some()),
            accepted_bytes: self.accepted_bytes, pending_records: self.pending.len(),
            pending_bytes: self.pending_bytes, retained_receipts: self.ready.len(),
        }
    }
    fn fits(&self, bytes: usize) -> bool {
        self.accounting().accepted_records < self.max_records
            && bytes <= self.max_bytes - self.accepted_bytes
    }
    fn id(&mut self) -> Result<u64, Error> {
        self.next = self.next.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(self.next)
    }
    fn commit(&mut self, id: u64, text: String) {
        self.accepted_bytes += text.len();
        self.records.push_back(Record { id, text });
    }
    fn promote(&mut self) {
        if self.sealed || self.failure.is_some() { return; }
        while self.pending.front().is_some_and(|p| self.fits(p.text.len())) {
            let p = self.pending.pop_front().expect("checked head");
            self.pending_bytes -= p.text.len();
            self.commit(p.ticket.id, p.text);
            self.ready.push_back(Ready { ticket: p.ticket, result: Ok(()) });
        }
    }
    /// String is borrowed until admission succeeds; copy only after budgets
    /// are checked. Rust str already guarantees well-formed UTF-8.
    pub fn admit(&mut self, owner: Owner, text: &str) -> Result<Admission, Error> {
        if owner.execution == 0 || owner.generation == 0 { return Err(Error::InvalidOwner); }
        if let Some(e) = self.failure { return Err(e); }
        if self.sealed { return Err(Error::Closed); }
        if text.len() > 65_536 || text.len() > self.max_bytes { return Err(Error::Oversize); }
        if self.pending.is_empty() && self.fits(text.len()) {
            let id = self.id()?;
            self.commit(id, text.to_owned());
            return Ok(Admission::Accepted);
        }
        // Pending AND unconsumed ready receipts share one bound, preventing
        // abandoned scheduler wakeups from growing the host queue.
        if self.pending.len() + self.ready.len() >= self.max_waiters
            || text.len() > self.max_pending - self.pending_bytes {
            return Err(Error::Quota);
        }
        let ticket = Ticket { descriptor: self.descriptor, id: self.id()?, owner };
        self.pending_bytes += text.len();
        self.pending.push_back(Pending { ticket, text: text.to_owned() });
        Ok(Admission::Pending(ticket))
    }
    pub fn take_ready(&mut self, owner: Owner, ticket: Ticket) -> Result<Option<Result<(), Error>>, Error> {
        if ticket.owner != owner { return Err(Error::Stale); }
        if let Some(i) = self.ready.iter().position(|r| r.ticket == ticket) {
            return Ok(Some(self.ready.remove(i).expect("checked receipt").result));
        }
        if self.pending.iter().any(|p| p.ticket == ticket) { return Ok(None); }
        Err(Error::Stale)
    }
    /// Cancellation can win only before commitment. It cannot retract output.
    pub fn cancel(&mut self, owner: Owner, ticket: Ticket) -> bool {
        if ticket.owner != owner { return false; }
        let Some(i) = self.pending.iter().position(|p| p.ticket == ticket) else { return false; };
        let p = self.pending.remove(i).expect("checked waiter");
        self.pending_bytes -= p.text.len();
        self.promote();
        true
    }
    /// Trusted retirement of the exact generation. Previously committed
    /// output remains accepted, but abandoned readiness cannot resume it.
    pub fn retire(&mut self, owner: Owner) {
        self.pending.retain(|p| {
            if p.ticket.owner == owner { self.pending_bytes -= p.text.len(); false } else { true }
        });
        self.ready.retain(|r| r.ticket.owner != owner);
        self.promote();
    }
    /// Transfer a single record to the trusted independent sink. Its charge
    /// remains live until finish_sink, including during physical writes.
    pub fn take_sink(&mut self) -> Option<(SinkTicket, String)> {
        if self.active.is_some() || self.failure.is_some() { return None; }
        let record = self.records.pop_front()?;
        self.active = Some((record.id, record.text.len()));
        Some((SinkTicket { descriptor: self.descriptor, id: record.id }, record.text))
    }
    pub fn finish_sink(&mut self, ticket: SinkTicket, success: bool) -> Result<(), Error> {
        let Some((active, bytes)) = self.active else { return Err(Error::Stale); };
        if ticket.descriptor != self.descriptor || active != ticket.id { return Err(Error::Stale); }
        self.active = None;
        self.accepted_bytes -= bytes;
        if !success { self.abort(); return Err(Error::SinkFailed); }
        self.promote();
        Ok(())
    }
    pub fn seal(&mut self) {
        self.sealed = true;
        while let Some(p) = self.pending.pop_front() {
            self.pending_bytes -= p.text.len();
            self.ready.push_back(Ready { ticket: p.ticket, result: Err(Error::Closed) });
        }
    }
    /// Active sink payload remains charged until the actual sink exit. No
    /// further records may start; physical bytes cannot be retracted.
    pub fn abort(&mut self) {
        if self.failure.is_some() { return; }
        self.failure = Some(Error::SinkFailed);
        self.sealed = true;
        while let Some(p) = self.pending.pop_front() {
            self.pending_bytes -= p.text.len();
            self.ready.push_back(Ready { ticket: p.ticket, result: Err(Error::SinkFailed) });
        }
        self.records.clear();
        self.accepted_bytes = self.active.map_or(0, |(_, bytes)| bytes);
    }
    pub fn drained(&self) -> bool { self.active.is_none() && self.records.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: Owner = Owner { execution: 1, generation: 1 };
    const B: Owner = Owner { execution: 1, generation: 2 };
    fn ticket(value: Admission) -> Ticket {
        match value { Admission::Pending(t) => t, _ => panic!("expected pending") }
    }
    #[test]
    fn fifo_capacity_cancel_and_exact_generation() {
        let mut q = BufferedStdio::new(4, 4, 4, 8).unwrap();
        assert_eq!(q.admit(A, "123"), Ok(Admission::Accepted));
        let (id, text) = q.take_sink().unwrap();
        assert_eq!(text, "123");
        let a = ticket(q.admit(A, "XX").unwrap());
        let b = ticket(q.admit(B, "Y").unwrap());
        assert_eq!(q.take_ready(B, b), Ok(None));
        assert!(!q.cancel(B, a));
        assert!(q.cancel(A, a));
        assert_eq!(q.take_ready(A, b), Err(Error::Stale));
        assert_eq!(q.take_ready(B, b), Ok(Some(Ok(()))));
        assert_eq!(q.take_ready(B, b), Err(Error::Stale));
        assert!(!q.cancel(B, b));
        assert_eq!(q.accounting().accepted_bytes, 4);
        assert_eq!(q.finish_sink(SinkTicket { id: id.id + 1, ..id }, true), Err(Error::Stale));
        q.finish_sink(id, true).unwrap();
        let (id, text) = q.take_sink().unwrap();
        assert_eq!(text, "Y");
        q.finish_sink(id, true).unwrap();
        assert!(q.drained());
    }
    #[test]
    fn ready_receipts_and_waiting_share_budget() {
        let mut q = BufferedStdio::new(1, 4, 1, 4).unwrap();
        q.admit(A, "éé").unwrap();
        let a = ticket(q.admit(A, "x").unwrap());
        let (id, _) = q.take_sink().unwrap();
        q.finish_sink(id, true).unwrap();
        assert_eq!(q.admit(B, "y"), Err(Error::Quota));
        q.retire(A);
        assert_eq!(q.take_ready(A, a), Err(Error::Stale));
        let b = ticket(q.admit(B, "y").unwrap());
        q.retire(A);
        assert_eq!(q.take_ready(B, b), Ok(None));
        let (id, _) = q.take_sink().unwrap();
        q.finish_sink(id, true).unwrap();
        assert_eq!(q.take_ready(B, b), Ok(Some(Ok(()))));
    }
    #[test]
    fn abort_retains_inflight_charge_and_rejects_pending() {
        let mut q = BufferedStdio::new(1, 4, 2, 4).unwrap();
        q.admit(A, "éé").unwrap();
        let (id, _) = q.take_sink().unwrap();
        let a = ticket(q.admit(A, "x").unwrap());
        q.abort();
        assert_eq!(q.take_ready(A, a), Ok(Some(Err(Error::SinkFailed))));
        assert_eq!(q.accounting().accepted_bytes, 4);
        assert!(!q.drained());
        assert_eq!(q.admit(A, "y"), Err(Error::SinkFailed));
        q.finish_sink(id, true).unwrap();
        assert!(q.drained());
        assert_eq!(q.accounting().accepted_bytes, 0);
        assert_eq!(q.finish_sink(id, true), Err(Error::Stale));
    }
    #[test]
    fn receipt_cannot_cross_descriptors() {
        let mut a = BufferedStdio::new(1, 4, 2, 4).unwrap();
        let mut b = BufferedStdio::new(1, 4, 2, 4).unwrap();
        a.admit(A, "x").unwrap(); b.admit(A, "x").unwrap();
        let ta = ticket(a.admit(A, "a").unwrap());
        let tb = ticket(b.admit(A, "b").unwrap());
        assert_ne!(ta, tb);
        assert!(!b.cancel(A, ta));
        assert_eq!(b.take_ready(A, ta), Err(Error::Stale));
        assert_eq!(b.take_ready(A, tb), Ok(None));
        let (sa, _) = a.take_sink().unwrap();
        let (sb, _) = b.take_sink().unwrap();
        assert_eq!(b.finish_sink(sa, true), Err(Error::Stale));
        b.finish_sink(sb, true).unwrap();
        a.finish_sink(sa, true).unwrap();
    }
    #[test]
    fn seal_and_limits() {
        let mut q = BufferedStdio::new(1, 4, 2, 2).unwrap();
        assert_eq!(q.admit(A, "😀x"), Err(Error::Oversize));
        q.admit(A, "x").unwrap();
        let a = ticket(q.admit(A, "yy").unwrap());
        assert_eq!(q.admit(B, "z"), Err(Error::Quota));
        q.seal();
        assert_eq!(q.take_ready(A, a), Ok(Some(Err(Error::Closed))));
        assert_eq!(q.admit(B, "z"), Err(Error::Closed));
        let (id, text) = q.take_sink().unwrap();
        assert_eq!(text, "x");
        q.finish_sink(id, true).unwrap();
        assert!(q.drained());
    }
}
