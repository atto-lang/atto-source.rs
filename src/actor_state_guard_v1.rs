//! Bounded, FIFO admission model for runtime-owned guarded actor state.
//!
//! This is a *logical* state machine, not a shared-memory allocator or lock.
//! A trusted runtime must serialize calls, authenticate actor identities,
//! install/retire physical guards, and establish physical quiescence before
//! invoking \`complete_physical_quiescence\`. Guest code gets no access to this API.
use std::collections::{BTreeMap, VecDeque};

pub const MAX_OUTSTANDING: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardError {
    InvalidIdentity,
    StaleGeneration,
    Retired,
    Capacity,
    Reentrant,
    TicketExhausted,
    UnknownTicket,
    WrongHolder,
    WriteRequiresExclusive,
    NotRetired,
    PhysicalQuiescenceRequired,
}

#[derive(Clone, Copy, Debug)]
struct Record {
    actor: u64,
    generation: u64,
    mode: Mode,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    ticket: u64,
    record: Record,
}

/// A non-owning, non-clonable permission; never a pointer to actor memory.
#[must_use]
#[derive(Debug)]
pub struct Permit {
    ticket: u64,
    record: Record,
}

impl Permit {
    pub fn ticket(&self) -> u64 {
        self.ticket
    }
    pub fn actor(&self) -> u64 {
        self.record.actor
    }
    pub fn generation(&self) -> u64 {
        self.record.generation
    }
    pub fn mode(&self) -> Mode {
        self.record.mode
    }
}

/// Serial admission service for one runtime-owned shared region.
///
/// The type contains no references to guest objects. Calls must be serialized
/// by the host; exposing this structure to an actor is not authorization.
#[derive(Debug)]
pub struct GuardQueue {
    generation: u64,
    next_ticket: u64,
    waiting: VecDeque<Pending>,
    active: BTreeMap<u64, Record>,
    retired: bool,
}

impl GuardQueue {
    pub fn new(generation: u64) -> Result<Self, GuardError> {
        if generation == 0 {
            return Err(GuardError::InvalidIdentity);
        }
        Ok(Self {
            generation,
            next_ticket: 1,
            waiting: VecDeque::new(),
            active: BTreeMap::new(),
            retired: false,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn waiting_count(&self) -> usize {
        self.waiting.len()
    }
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
    pub fn is_retired(&self) -> bool {
        self.retired
    }

    fn check_open(&self, actor: u64, generation: u64) -> Result<(), GuardError> {
        if actor == 0 || generation == 0 {
            return Err(GuardError::InvalidIdentity);
        }
        if generation != self.generation {
            return Err(GuardError::StaleGeneration);
        }
        if self.retired {
            return Err(GuardError::Retired);
        }
        Ok(())
    }

    /// Enqueue in arrival order. A non-reentrant actor may have at most one
    /// outstanding request or permit in a region.
    pub fn request(&mut self, actor: u64, generation: u64, mode: Mode) -> Result<u64, GuardError> {
        self.check_open(actor, generation)?;
        if self.waiting.iter().any(|p| p.record.actor == actor)
            || self.active.values().any(|p| p.actor == actor)
        {
            return Err(GuardError::Reentrant);
        }
        if self.waiting.len() + self.active.len() >= MAX_OUTSTANDING {
            return Err(GuardError::Capacity);
        }
        let next = self
            .next_ticket
            .checked_add(1)
            .ok_or(GuardError::TicketExhausted)?;
        let ticket = self.next_ticket;
        self.next_ticket = next;
        self.waiting.push_back(Pending {
            ticket,
            record: Record {
                actor,
                generation,
                mode,
            },
        });
        Ok(ticket)
    }

    /// Admit the oldest writer, or a contiguous prefix of readers.
    ///
    /// A later reader can never overtake a queued writer. This function
    /// returns immediately; it never blocks a carrier thread.
    pub fn admit_ready(&mut self) -> Vec<Permit> {
        if self.retired || self.active.values().any(|p| p.mode == Mode::Write) {
            return Vec::new();
        }
        let mut granted = Vec::new();
        while let Some(front) = self.waiting.front().copied() {
            if front.record.mode == Mode::Write && !self.active.is_empty() {
                break;
            }
            let pending = self.waiting.pop_front().expect("front was present");
            self.active.insert(pending.ticket, pending.record);
            granted.push(Permit {
                ticket: pending.ticket,
                record: pending.record,
            });
            if pending.record.mode == Mode::Write {
                break;
            }
        }
        granted
    }

    fn authenticate(&self, permit: &Permit, actor: u64) -> Result<Record, GuardError> {
        if actor == 0 || permit.record.actor != actor {
            return Err(GuardError::WrongHolder);
        }
        let live = *self.active.get(&permit.ticket).ok_or(GuardError::UnknownTicket)?;
        if live.actor != permit.record.actor
            || live.generation != permit.record.generation
            || live.mode != permit.record.mode
        {
            return Err(GuardError::WrongHolder);
        }
        Ok(live)
    }

    /// Validates a logical read or write; this never returns a raw memory ref.
    pub fn access(&self, permit: &Permit, actor: u64, wants_write: bool) -> Result<(), GuardError> {
        let live = self.authenticate(permit, actor)?;
        if self.retired || live.generation != self.generation {
            return Err(GuardError::StaleGeneration);
        }
        if wants_write && live.mode != Mode::Write {
            return Err(GuardError::WriteRequiresExclusive);
        }
        Ok(())
    }

    /// End a live scope. A stale or retired scope may not claim physical
    /// quiescence through this ordinary exit path.
    pub fn release(&mut self, permit: &Permit, actor: u64) -> Result<(), GuardError> {
        self.authenticate(permit, actor)?;
        if self.retired {
            return Err(GuardError::PhysicalQuiescenceRequired);
        }
        self.active.remove(&permit.ticket);
        Ok(())
    }

    /// Cancel only a queued request; admitted permits need a proper exit or
    /// externally authenticated physical-quiescence completion.
    pub fn cancel_pending(
        &mut self,
        ticket: u64,
        actor: u64,
        generation: u64,
    ) -> Result<(), GuardError> {
        self.check_open(actor, generation)?;
        let at = self
            .waiting
            .iter()
            .position(|p| p.ticket == ticket)
            .ok_or(GuardError::UnknownTicket)?;
        if self.waiting[at].record.actor != actor {
            return Err(GuardError::WrongHolder);
        }
        self.waiting.remove(at);
        Ok(())
    }

    /// Fence STOP/hot-upgrade: invalidate all logical access, remove all
    /// waiting requests, but retain backing ownership for active permits.
    pub fn retire(&mut self, next_generation: u64) -> Result<(), GuardError> {
        if next_generation <= self.generation {
            return Err(GuardError::StaleGeneration);
        }
        if self.retired {
            return Err(GuardError::Retired);
        }
        self.generation = next_generation;
        self.retired = true;
        self.waiting.clear();
        Ok(())
    }

    /// TRUSTED RUNTIME ONLY: call after independently proving that callbacks,
    /// carriers and guest accesses for this permit have physically stopped.
    /// A stale guest acknowledgement is not such a proof.
    pub fn complete_physical_quiescence(
        &mut self,
        permit: &Permit,
        actor: u64,
    ) -> Result<(), GuardError> {
        if !self.retired {
            return Err(GuardError::NotRetired);
        }
        self.authenticate(permit, actor)?;
        self.active.remove(&permit.ticket);
        Ok(())
    }

    /// Only a retired, fully quiesced region may be reclaimed. This is a
    /// necessary model condition, not an allocation/refcount or heap proof.
    pub fn may_reclaim(&self) -> bool {
        self.retired && self.active.is_empty() && self.waiting.is_empty()
    }
}
