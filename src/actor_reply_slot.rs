//! Bounded incarnation-fenced one-shot reply primitive.
//! HOST_ONLY until the actor mailbox, scheduler and Wasm guest ABI use it.
//! Caller serializes mutation at the actor runtime's admission boundary.
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFailure { MissingReply, HandlerFailed, Cancelled, TimedOut, ActorRetired }
#[derive(Debug, PartialEq, Eq)]
pub enum ReplyOutcome<T> { Value(T), Failure(ReplyFailure) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotFault { Full, Stale, AlreadyStaged, AlreadySettled, NotSettled, InvalidDeadline }

/// Private correlation evidence, not a guest payload or remote authentication.
#[derive(Debug, PartialEq, Eq)]
pub struct Ticket {
    index: usize,
    generation: u64,
    caller_incarnation: u64,
    callee_incarnation: u64,
}
impl Ticket {
    /// Create one separate dispatcher permit for a trusted, in-process
    /// mailbox admission. This is NOT a public Clone implementation and
    /// is never serialized to untrusted guest code.
    pub(crate) fn for_dispatch(&self) -> Self {
        Self {
            index: self.index,
            generation: self.generation,
            caller_incarnation: self.caller_incarnation,
            callee_incarnation: self.callee_incarnation,
        }
    }
}
enum Phase<T> { Free, Open, Staged(T), Settled(ReplyOutcome<T>), PermanentlyRetired }
struct Slot<T> {
    generation: u64,
    caller_incarnation: u64,
    callee_incarnation: u64,
    deadline: u64,
    phase: Phase<T>,
}
impl<T> Slot<T> {
    fn new() -> Self {
        Self { generation: 1, caller_incarnation: 0, callee_incarnation: 0,
            deadline: 0, phase: Phase::Free }
    }
}
/// Fixed-size reply registry; callers must synchronize admission/settlement.
pub struct ReplyRegistry<T> {
    slots: Vec<Slot<T>>,
    callee_incarnation: u64,
    completed_order: VecDeque<usize>,
}
impl<T> ReplyRegistry<T> {
    pub fn new(capacity: usize, callee_incarnation: u64) -> Self {
        assert!(capacity > 0);
        Self {
            slots: (0..capacity).map(|_| Slot::new()).collect(),
            callee_incarnation, completed_order: VecDeque::with_capacity(capacity),
        }
    }
    pub fn capacity(&self) -> usize { self.slots.len() }
    pub fn available(&self) -> usize {
        self.slots.iter().filter(|s| matches!(s.phase, Phase::Free)).count()
    }
    /// Admission must be atomic with mailbox enqueue in a future runtime.
    pub fn admit(&mut self, caller_incarnation: u64, deadline: u64, now: u64)
        -> Result<Ticket, SlotFault> {
        if deadline <= now { return Err(SlotFault::InvalidDeadline); }
        let (index, slot) = self.slots.iter_mut().enumerate()
            .find(|(_, s)| matches!(s.phase, Phase::Free))
            .ok_or(SlotFault::Full)?;
        slot.phase = Phase::Open;
        slot.caller_incarnation = caller_incarnation;
        slot.callee_incarnation = self.callee_incarnation;
        slot.deadline = deadline;
        Ok(Ticket {
            index, generation: slot.generation, caller_incarnation,
            callee_incarnation: self.callee_incarnation,
        })
    }
    fn current(&mut self, t: &Ticket) -> Result<&mut Slot<T>, SlotFault> {
        let slot = self.slots.get_mut(t.index).ok_or(SlotFault::Stale)?;
        if slot.generation != t.generation ||
            slot.caller_incarnation != t.caller_incarnation ||
            slot.callee_incarnation != t.callee_incarnation ||
            matches!(slot.phase, Phase::Free | Phase::PermanentlyRetired) {
            return Err(SlotFault::Stale);
        }
        Ok(slot)
    }
    /// Both return R and respond_with(R) eventually lower to this one slot.
    /// Staging is invisible to callers until successful turn completion.
    pub fn stage(&mut self, t: &Ticket, value: T, now: u64) -> Result<(), SlotFault> {
        let s = self.current(t)?;
        if now >= s.deadline { return Err(SlotFault::AlreadySettled); }
        match s.phase {
            Phase::Open => { s.phase = Phase::Staged(value); Ok(()) }
            Phase::Staged(_) => Err(SlotFault::AlreadyStaged),
            Phase::Settled(_) => Err(SlotFault::AlreadySettled),
            _ => Err(SlotFault::Stale),
        }
    }
    fn settle(&mut self, t: &Ticket, outcome: ReplyOutcome<T>) -> Result<(), SlotFault> {
        let s = self.current(t)?;
        if matches!(s.phase, Phase::Settled(_)) { return Err(SlotFault::AlreadySettled); }
        s.phase = Phase::Settled(outcome);
        self.completed_order.push_back(t.index);
        Ok(())
    }
    pub fn finish_turn(&mut self, t: &Ticket, now: u64) -> Result<(), SlotFault> {
        let s = self.current(t)?;
        let outcome = if now >= s.deadline {
            ReplyOutcome::Failure(ReplyFailure::TimedOut)
        } else {
            match std::mem::replace(&mut s.phase, Phase::Open) {
                Phase::Staged(value) => ReplyOutcome::Value(value),
                Phase::Open => ReplyOutcome::Failure(ReplyFailure::MissingReply),
                Phase::Settled(previous) => {
                    s.phase = Phase::Settled(previous);
                    return Err(SlotFault::AlreadySettled);
                }
                other => { s.phase = other; return Err(SlotFault::Stale); }
            }
        };
        self.settle(t, outcome)
    }
    /// Failure discards any staged reply; supervisor CONTROL is independent.
    pub fn fail_turn(&mut self, t: &Ticket) -> Result<(), SlotFault> {
        self.settle(t, ReplyOutcome::Failure(ReplyFailure::HandlerFailed))
    }
    pub fn cancel(&mut self, t: &Ticket) -> Result<(), SlotFault> {
        self.settle(t, ReplyOutcome::Failure(ReplyFailure::Cancelled))
    }
    pub fn expire(&mut self, now: u64) -> usize {
        let mut count = 0;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if matches!(slot.phase, Phase::Open | Phase::Staged(_)) && now >= slot.deadline {
                slot.phase = Phase::Settled(ReplyOutcome::Failure(ReplyFailure::TimedOut));
                self.completed_order.push_back(i);
                count += 1;
            }
        }
        count
    }
    /// Retiring an actor rejects late replies while preserving pending
    /// callers' ability to consume a terminal failure from their old tickets.
    pub fn retire_actor(&mut self, next_incarnation: u64) -> Result<usize, SlotFault> {
        if next_incarnation <= self.callee_incarnation { return Err(SlotFault::Stale); }
        let mut count = 0;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if matches!(slot.phase, Phase::Open | Phase::Staged(_)) {
                slot.phase = Phase::Settled(ReplyOutcome::Failure(ReplyFailure::ActorRetired));
                self.completed_order.push_back(i);
                count += 1;
            }
        }
        self.callee_incarnation = next_incarnation;
        Ok(count)
    }
    pub fn poll(&self, t: &Ticket) -> Result<Option<&ReplyOutcome<T>>, SlotFault> {
        let slot = self.slots.get(t.index).ok_or(SlotFault::Stale)?;
        if slot.generation != t.generation ||
            slot.caller_incarnation != t.caller_incarnation ||
            slot.callee_incarnation != t.callee_incarnation {
            return Err(SlotFault::Stale);
        }
        match &slot.phase {
            Phase::Open | Phase::Staged(_) => Ok(None),
            Phase::Settled(result) => Ok(Some(result)),
            Phase::Free | Phase::PermanentlyRetired => Err(SlotFault::Stale),
        }
    }
    /// Consuming exactly once increments generation, fencing ABA reuse.
    /// Generation exhaustion permanently tombstones the slot, never wraps.
    pub fn receive(&mut self, t: &Ticket) -> Result<ReplyOutcome<T>, SlotFault> {
        let s = self.current(t)?;
        if !matches!(s.phase, Phase::Settled(_)) { return Err(SlotFault::NotSettled); }
        let result = match std::mem::replace(&mut s.phase, Phase::Free) {
            Phase::Settled(v) => v,
            _ => unreachable!("checked settled phase"),
        };
        if let Some(next) = s.generation.checked_add(1) {
            s.generation = next;
        } else {
            s.phase = Phase::PermanentlyRetired;
        }
        self.completed_order.retain(|index| *index != t.index);
        Ok(result)
    }
    pub fn completed_count(&self) -> usize { self.completed_order.len() }
}
