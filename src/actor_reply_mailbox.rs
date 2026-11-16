//! Atomic bounded unary mailbox and reply-slot admission (host core).
//! This is a scheduler primitive, not an executable AttoLang guest ABI.
//! Trusted host owns both queue and registry; no CONTROL notices or tickets
//! are serialized to guests. Caller must serialize access to this type.
use std::collections::VecDeque;
use crate::actor_reply_slot::{ReplyOutcome, ReplyRegistry, SlotFault, Ticket};

#[derive(Debug, PartialEq, Eq)]
pub enum MailFault { Closed, QueueFull, Reply(SlotFault) }
impl From<SlotFault> for MailFault {
    fn from(value: SlotFault) -> Self { Self::Reply(value) }
}

/// Caller-only receipt does not grant the right to stage a reply.
#[derive(Debug, PartialEq, Eq)]
pub struct ReplyReceipt { ticket: Ticket }

/// Trusted, non-forgeable delivery permit, obtained only from admitted queue.
#[derive(Debug)]
pub struct Delivery<M> { payload: M, reply_permit: Ticket }
impl<M> Delivery<M> {
    pub fn payload(&self) -> &M { &self.payload }
}

/// Fixed bounds on pending mail and all outstanding replies, including inflight.
pub struct UnaryMailbox<M, R> {
    queue: VecDeque<Delivery<M>>,
    reply: ReplyRegistry<R>,
    max_queued: usize,
    closed: bool,
}
impl<M, R> UnaryMailbox<M, R> {
    pub fn new(max_queued: usize, max_replies: usize, callee_incarnation: u64) -> Self {
        assert!(max_queued > 0 && max_replies > 0);
        Self { queue: VecDeque::with_capacity(max_queued),
            reply: ReplyRegistry::new(max_replies, callee_incarnation),
            max_queued, closed: false }
    }
    pub fn pending(&self) -> usize { self.queue.len() }
    pub fn outstanding_capacity(&self) -> usize { self.reply.capacity() }
    pub fn reply_slots_free(&self) -> usize { self.reply.available() }
    pub fn is_closed(&self) -> bool { self.closed }

    /// Both queue admission and reply slot assignment succeed or neither does.
    pub fn admit(&mut self, payload: M, caller_incarnation: u64,
        deadline: u64, now: u64) -> Result<ReplyReceipt, MailFault> {
        if self.closed { return Err(MailFault::Closed); }
        // Reclaim expired/cancelled queued entries even if no dispatcher has
        // polled yet. Their terminal receipts remain collectable by callers.
        self.reply.expire(now);
        let reply = &self.reply;
        self.queue.retain(|d| matches!(reply.poll(&d.reply_permit), Ok(None)));
        if self.queue.len() >= self.max_queued { return Err(MailFault::QueueFull); }
        let ticket = self.reply.admit(caller_incarnation, deadline, now)?;
        self.queue.push_back(Delivery {
            payload, reply_permit: ticket.for_dispatch(),
        });
        Ok(ReplyReceipt { ticket })
    }

    /// Pending requests already settled by cancellation or timeout are
    /// discarded rather than executed. Inflight deliveries consume capacity.
    pub fn next(&mut self, now: u64) -> Option<Delivery<M>> {
        // No expired request can be dispatched, even if the runtime timer
        // has not run since the deadline. This sweep is capacity-bounded.
        self.reply.expire(now);
        while let Some(delivery) = self.queue.pop_front() {
            if matches!(self.reply.poll(&delivery.reply_permit), Ok(None)) {
                return Some(delivery);
            }
        }
        None
    }

    /// A return value and respond_with use the same write-once staging gate.
    pub fn stage(&mut self, delivery: &Delivery<M>, response: R,
        now: u64) -> Result<(), SlotFault> {
        // Deadline must be enforced at staging, not only at the timer tick.
        self.reply.expire(now);
        self.reply.stage(&delivery.reply_permit, response, now)
    }

    /// Complete the actor turn; staged value publishes exactly once.
    pub fn complete(&mut self, delivery: Delivery<M>, now: u64)
        -> Result<(), SlotFault> {
        self.reply.finish_turn(&delivery.reply_permit, now)
    }

    /// Panic/raise/throw discards staging and terminates with a reply error.
    pub fn fail(&mut self, delivery: Delivery<M>) -> Result<(), SlotFault> {
        self.reply.fail_turn(&delivery.reply_permit)
    }

    pub fn cancel(&mut self, receipt: &ReplyReceipt) -> Result<(), SlotFault> {
        self.reply.cancel(&receipt.ticket)
    }
    pub fn poll(&self, receipt: &ReplyReceipt)
        -> Result<Option<&ReplyOutcome<R>>, SlotFault> {
        self.reply.poll(&receipt.ticket)
    }
    /// Consume exactly once and fence old delivery by bumping generation.
    pub fn collect(&mut self, receipt: ReplyReceipt)
        -> Result<ReplyOutcome<R>, SlotFault> {
        self.reply.receive(&receipt.ticket)
    }

    pub fn expire(&mut self, now: u64) -> usize { self.reply.expire(now) }

    /// Close admission, discard pending application mail and fail every
    /// outstanding reply while keeping supervisor priority CONTROL separate.
    pub fn retire(&mut self, next_incarnation: u64) -> Result<usize, SlotFault> {
        let n = self.reply.retire_actor(next_incarnation)?;
        self.closed = true;
        self.queue.clear();
        Ok(n)
    }

    /// Caller must pass a strictly higher actor incarnation to retire.
    /// Reopening never revives stale tickets.
    pub fn reopen(&mut self) { self.closed = false; }
}
