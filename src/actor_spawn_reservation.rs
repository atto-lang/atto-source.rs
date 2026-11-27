//! Fixed-capacity host reservation substrate, not guest allocation or AOT lowering.
//! The trusted scheduler serializes mutations (e.g. under its own mutex).
//! Logical leases survive carrier suspension; callers must separately prove physical
//! task, FFI and allocator quiescence before release.
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Charge([u64; 4]);
impl Charge {
    pub fn new(heap: u64, stack: u64, mailbox_bytes: u64, messages: u64) -> Result<Self, Error> {
        let values = [heap, stack, mailbox_bytes, messages];
        if values.contains(&0) { return Err(Error::Quota); }
        Ok(Self(values))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Quota, Full, Exhausted, Stale, InitAuthority, Retiring, NotRetired, InFlight }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Initializing, Ready, Retiring }
#[derive(Clone, Copy)]
struct Entry<const R: usize> {
    generation: u64,
    charge: Charge,
    phase: Phase,
    init_started: bool,
    init_live: bool,
    receives: [Option<u64>; R],
    next_receive: u64,
}
// Private, nonforgeable host identity. A stale permit keeps the original
// identity alive, preventing allocator-address reuse from authenticating it.
#[derive(Clone, Debug)]
pub struct Permit { owner: Arc<()>, slot: usize, generation: u64 }
impl Permit { pub fn generation(&self) -> u64 { self.generation } }
#[derive(Debug)]
pub struct InitLease { permit: Permit }
#[derive(Debug)]
pub struct ReceiveLease { permit: Permit, id: u64 }

pub struct Gate<const N: usize, const R: usize> {
    identity: Arc<()>,
    limit: Charge,
    used: [u64; 4],
    generation: u64,
    entries: [Option<Entry<R>>; N],
}
impl<const N: usize, const R: usize> Gate<N, R> {
    pub fn new(limit: Charge) -> Self {
        Self { identity: Arc::new(()), limit, used: [0; 4],
            generation: 0, entries: [None; N] }
    }
    pub fn usage(&self) -> [u64; 4] { self.used }
    pub fn reserve(&mut self, charge: Charge) -> Result<Permit, Error> {
        if (0..4).any(|i| charge.0[i] > self.limit.0[i] - self.used[i]) {
            return Err(Error::Quota);
        }
        let slot = self.entries.iter().position(Option::is_none).ok_or(Error::Full)?;
        let generation = self.generation.checked_add(1).ok_or(Error::Exhausted)?;
        let permit = Permit { owner: Arc::clone(&self.identity), slot, generation };
        self.entries[slot] = Some(Entry { generation, charge, phase: Phase::Initializing,
            init_started: false, init_live: false, receives: [None; R], next_receive: 0 });
        for i in 0..4 { self.used[i] += charge.0[i]; }
        self.generation = generation;
        Ok(permit)
    }
    fn entry(&self, p: &Permit) -> Result<&Entry<R>, Error> {
        if !Arc::ptr_eq(&p.owner, &self.identity) { return Err(Error::Stale); }
        self.entries.get(p.slot).and_then(Option::as_ref)
            .filter(|e| e.generation == p.generation).ok_or(Error::Stale)
    }
    fn entry_mut(&mut self, p: &Permit) -> Result<&mut Entry<R>, Error> {
        self.entry(p)?;
        self.entries[p.slot].as_mut().ok_or(Error::Stale)
    }
    pub fn begin_init(&mut self, p: &Permit) -> Result<InitLease, Error> {
        let e = self.entry_mut(p)?;
        if e.phase != Phase::Initializing || e.init_started { return Err(Error::InitAuthority); }
        e.init_started = true;
        e.init_live = true;
        Ok(InitLease { permit: p.clone() })
    }
    pub fn complete_init(&mut self, lease: &InitLease, success: bool) -> Result<bool, Error> {
        let e = self.entry_mut(&lease.permit)?;
        if !e.init_live { return Err(Error::InitAuthority); }
        e.init_live = false;
        if e.phase == Phase::Retiring || !success {
            e.phase = Phase::Retiring;
            return Ok(false);
        }
        e.phase = Phase::Ready;
        Ok(true)
    }
    pub fn may_receive(&self, p: &Permit) -> bool {
        self.entry(p).is_ok_and(|e| e.phase == Phase::Ready)
    }
    pub fn try_enter_receive(&mut self, p: &Permit) -> Result<Option<ReceiveLease>, Error> {
        let e = self.entry_mut(p)?;
        if e.phase != Phase::Ready { return Ok(None); }
        let slot = e.receives.iter().position(Option::is_none).ok_or(Error::Full)?;
        let id = e.next_receive.checked_add(1).ok_or(Error::Exhausted)?;
        e.receives[slot] = Some(id);
        e.next_receive = id;
        Ok(Some(ReceiveLease { permit: p.clone(), id }))
    }
    pub fn finish_receive(&mut self, lease: &ReceiveLease) -> Result<(), Error> {
        let e = self.entry_mut(&lease.permit)?;
        let slot = e.receives.iter_mut().find(|s| **s == Some(lease.id)).ok_or(Error::Stale)?;
        *slot = None;
        Ok(())
    }
    pub fn retire(&mut self, p: &Permit) -> Result<(), Error> {
        self.entry_mut(p)?.phase = Phase::Retiring;
        Ok(())
    }
    pub fn release_after_quiescence(&mut self, p: &Permit) -> Result<(), Error> {
        let e = self.entry(p)?;
        if e.phase != Phase::Retiring { return Err(Error::NotRetired); }
        if e.init_live || e.receives.iter().any(Option::is_some) { return Err(Error::InFlight); }
        let charge = e.charge;
        self.entries[p.slot] = None;
        for i in 0..4 { self.used[i] -= charge.0[i]; }
        Ok(())
    }
}
