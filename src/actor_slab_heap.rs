//! Bounded, size-class slab allocator for one actor incarnation: the Rust and
//! wasm32 twin of atto-source.llvm `runtime/ActorSlabHeap.hpp`.
//!
//! A 4-byte actor reference only saves memory if the allocator's own metadata
//! does not eat the saving. Objects carry no per-object header; 4 KiB slab
//! pages hold one size class each, with 64 bytes reserved at the start of
//! every page for its header (the header lives out of band here, but the
//! space is kept so offsets match the C++ heap exactly).
//!
//! * [`Ref32`] (Offset32) is `offset / 8 + 1`, the cross-backend word: no
//!   per-object metadata, for compiler-proven actor-local lifetimes. It rejects
//!   null, foreign, interior, unallocated and freed words, not ABA.
//! * [`Handle32`] is `(generation << slot_bits) | (slot + 1)` through an
//!   8-byte table entry: slot bits are sized to capacity, slots are reused
//!   FIFO, and an exhausted slot is quarantined, never wrapped.
//!
//! The allocation decisions (page order, partial lists, first-fit free runs,
//! handle table placement, FIFO reuse) mirror the C++ heap, so the same
//! operation sequence yields the same words on every backend; the golden test
//! below enforces that. Memory is a `Vec<u8>` committed in granules up to a
//! budget; references are offsets, so growth never invalidates them. The
//! module is `unsafe`-free and portable to wasm32, where `usize` is 32 bits.
//! Every failure returns an error and leaves the heap unchanged.

use core::num::NonZeroU32;

pub const PAGE_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 64;
pub const MAX_SMALL: usize = 2048;
pub const MAX_ALIGNMENT: usize = 64;
pub const CLASS_BYTES: [u16; 24] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024, 1280, 1536, 1792, 2048,
];
const CLASSES: usize = CLASS_BYTES.len();
const BITMAP_BITS: usize = (HEADER_BYTES - 16) * 8; // same capacity as the C++ header
const NONE: u32 = 0; // page links and free lists are index + 1
/// Largest span a ref32 word can address: (2^32 - 1) quanta of 8 bytes.
const MAX_SPAN: u64 = (u32::MAX as u64) * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlabError {
    OutOfMemory,
    BudgetExceeded,
    InvalidSize,
    InvalidAlignment,
    InvalidRef,
    StaleHandle,
    HandleTableFull,
    InvalidConfig,
    Retired,
}

/// Offset32 actor reference: `offset / 8 + 1`. Zero (null) is unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Ref32(NonZeroU32);

impl Ref32 {
    /// Re-read a stored 4-byte field. NOT a proof: every use is validated.
    pub fn from_word(word: u32) -> Option<Self> {
        NonZeroU32::new(word).map(Ref32)
    }
    pub fn word(self) -> u32 {
        self.0.get()
    }
    fn from_offset(offset: usize) -> Option<Self> {
        if !offset.is_multiple_of(8) {
            return None;
        }
        let index = (offset / 8) as u64;
        if index >= u32::MAX as u64 {
            return None;
        }
        Self::from_word(index as u32 + 1)
    }
    fn offset(self) -> u64 {
        (self.0.get() as u64 - 1) * 8
    }
}

/// Handle32: generation-checked indirection for stale-reference protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Handle32(pub u32);

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub reserve_bytes: usize,
    pub budget_bytes: usize,
    pub commit_granule: usize,
    pub handle_capacity: u32,
    /// 0 = every bit not needed for the slot.
    pub handle_generation_bits: u32,
}

impl Config {
    pub fn new(reserve_bytes: usize, budget_bytes: usize) -> Self {
        Config {
            reserve_bytes,
            budget_bytes,
            commit_granule: 64 * 1024,
            handle_capacity: 0,
            handle_generation_bits: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub reserved_bytes: usize,
    pub committed_bytes: usize,
    pub metadata_bytes: usize,
    pub live_block_bytes: usize,
    pub live_objects: usize,
    pub live_pages: usize,
    pub free_pages: usize,
    pub quarantined_handles: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Unused,
    Small,
    Large,
    LargeTail,
    Free,
    FreeTail,
    Meta,
}

#[derive(Clone, Copy)]
struct Page {
    kind: Kind,
    cls: u8,
    live: u16,
    free_head: u16,
    bump: u16,
    prev: u32,
    next: u32,
    run: u32, // large / free-run / meta length in pages
    bitmap: [u64; BITMAP_BITS / 64],
}

impl Page {
    const EMPTY: Page = Page {
        kind: Kind::Unused,
        cls: 0,
        live: 0,
        free_head: 0,
        bump: 0,
        prev: NONE,
        next: NONE,
        run: 0,
        bitmap: [0; BITMAP_BITS / 64],
    };
    fn bit(&self, i: usize) -> bool {
        (self.bitmap[i / 64] >> (i % 64)) & 1 == 1
    }
    fn set_bit(&mut self, i: usize, on: bool) {
        if on {
            self.bitmap[i / 64] |= 1 << (i % 64)
        } else {
            self.bitmap[i / 64] &= !(1 << (i % 64))
        }
    }
}

fn blocks_in_page(size: usize) -> u16 {
    ((PAGE_BYTES - HEADER_BYTES) / size).min(BITMAP_BITS) as u16
}

fn class_for(bytes: usize, align: usize) -> Option<u8> {
    CLASS_BYTES
        .iter()
        .position(|&c| c as usize >= bytes && (c as usize).is_multiple_of(align))
        .map(|c| c as u8)
}

pub struct ActorSlabHeap {
    mem: Vec<u8>,
    reserve: usize,
    budget: usize,
    granule: usize,
    retired: bool,
    pages: Vec<Page>,
    max_pages: u32,
    high_water: u32,
    partial: [u32; CLASSES],
    free_runs: u32,
    handle_base: usize,
    handle_capacity: u32,
    slot_bits: u32,
    max_gen: u32,
    free_slot_head: u32,
    free_slot_tail: u32,
    quarantined: u32,
    stats: Stats,
}

impl ActorSlabHeap {
    pub fn new(config: Config) -> Result<Self, SlabError> {
        let reserve = config.reserve_bytes / PAGE_BYTES * PAGE_BYTES;
        if reserve == 0
            || config.budget_bytes == 0
            || config.budget_bytes > reserve
            || reserve as u64 > MAX_SPAN
            || reserve / PAGE_BYTES > u32::MAX as usize - 1
        {
            return Err(SlabError::InvalidConfig);
        }
        let max_pages = (reserve / PAGE_BYTES) as u32;
        let mut heap = ActorSlabHeap {
            mem: Vec::new(),
            reserve,
            budget: config.budget_bytes,
            granule: (config.commit_granule.max(PAGE_BYTES)) / PAGE_BYTES * PAGE_BYTES,
            retired: false,
            pages: Vec::new(),
            max_pages,
            high_water: 0,
            partial: [NONE; CLASSES],
            free_runs: NONE,
            handle_base: 0,
            handle_capacity: 0,
            slot_bits: 0,
            max_gen: 0,
            free_slot_head: NONE,
            free_slot_tail: NONE,
            quarantined: 0,
            stats: Stats {
                reserved_bytes: reserve,
                metadata_bytes: max_pages as usize,
                ..Stats::default()
            },
        };
        if config.handle_capacity != 0 {
            heap.init_handles(config.handle_capacity, config.handle_generation_bits)?;
        }
        Ok(heap)
    }

    fn init_handles(&mut self, capacity: u32, gen_bits_cfg: u32) -> Result<(), SlabError> {
        let mut bits = 0u32;
        while (1u64 << bits) <= capacity as u64 {
            bits += 1;
        }
        if bits >= 31 {
            return Err(SlabError::InvalidConfig);
        }
        let mut gen_bits = 32 - bits;
        if gen_bits_cfg != 0 {
            if gen_bits_cfg < 2 || gen_bits_cfg > gen_bits {
                return Err(SlabError::InvalidConfig);
            }
            gen_bits = gen_bits_cfg;
        }
        let table = capacity as usize * 8;
        let count = (table + HEADER_BYTES).div_ceil(PAGE_BYTES) as u32;
        let first = self.take_pages(count)?;
        self.pages[first as usize] = Page {
            kind: Kind::Meta,
            run: count,
            ..Page::EMPTY
        };
        for p in 1..count {
            self.pages[(first + p) as usize].kind = Kind::LargeTail;
        }
        self.handle_base = first as usize * PAGE_BYTES + HEADER_BYTES;
        self.handle_capacity = capacity;
        self.slot_bits = bits;
        self.max_gen = ((1u64 << gen_bits) - 1) as u32;
        for s in 0..capacity {
            let next = if s + 1 < capacity { s + 2 } else { NONE };
            self.write_entry(s, next, 1);
        }
        self.free_slot_head = 1;
        self.free_slot_tail = capacity;
        self.stats.metadata_bytes += count as usize * PAGE_BYTES;
        self.stats.live_pages += count as usize;
        Ok(())
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }
    pub fn handle_slot_bits(&self) -> u32 {
        self.slot_bits
    }
    pub fn handle_max_generation(&self) -> u32 {
        self.max_gen
    }

    // ---- memory -----------------------------------------------------------
    fn commit_through(&mut self, end: usize) -> Result<(), SlabError> {
        let committed = self.mem.len();
        if end <= committed {
            return Ok(());
        }
        if end > self.reserve {
            return Err(SlabError::OutOfMemory);
        }
        let mut target = (end.div_ceil(self.granule) * self.granule).min(self.reserve);
        if target > self.budget {
            if end > self.budget {
                return Err(SlabError::BudgetExceeded);
            }
            target = self.budget;
        }
        if self.mem.try_reserve_exact(target - committed).is_err() {
            return Err(SlabError::OutOfMemory);
        }
        self.mem.resize(target, 0);
        self.stats.committed_bytes = target;
        Ok(())
    }

    fn take_pages(&mut self, count: u32) -> Result<u32, SlabError> {
        let mut link = self.free_runs;
        while link != NONE {
            let first = link - 1;
            let have = self.pages[first as usize].run;
            if have >= count {
                self.unlink_free_run(first);
                if have > count {
                    self.push_free_run(first + count, have - count);
                }
                self.stats.free_pages -= count as usize;
                return Ok(first);
            }
            link = self.pages[first as usize].next;
        }
        if count > self.max_pages - self.high_water {
            return Err(SlabError::OutOfMemory);
        }
        let first = self.high_water;
        self.commit_through((first + count) as usize * PAGE_BYTES)?;
        self.high_water += count;
        if self.pages.len() < self.high_water as usize {
            self.pages.resize(self.high_water as usize, Page::EMPTY);
        }
        Ok(first)
    }

    fn push_free_run(&mut self, first: u32, mut count: u32) {
        let after = first + count;
        if after < self.high_water && self.pages[after as usize].kind == Kind::Free {
            let more = self.pages[after as usize].run;
            self.unlink_free_run(after);
            count += more;
        }
        self.pages[first as usize] = Page {
            kind: Kind::Free,
            run: count,
            next: self.free_runs,
            prev: NONE,
            ..Page::EMPTY
        };
        for p in 1..count {
            self.pages[(first + p) as usize].kind = Kind::FreeTail;
        }
        if self.free_runs != NONE {
            self.pages[(self.free_runs - 1) as usize].prev = first + 1;
        }
        self.free_runs = first + 1;
    }

    fn unlink_free_run(&mut self, first: u32) {
        let (prev, next) = (self.pages[first as usize].prev, self.pages[first as usize].next);
        if prev != NONE {
            self.pages[(prev - 1) as usize].next = next
        } else {
            self.free_runs = next
        }
        if next != NONE {
            self.pages[(next - 1) as usize].prev = prev;
        }
        self.pages[first as usize].kind = Kind::Unused;
    }

    fn partial_push(&mut self, cls: u8, index: u32) {
        let head = self.partial[cls as usize];
        self.pages[index as usize].prev = NONE;
        self.pages[index as usize].next = head;
        if head != NONE {
            self.pages[(head - 1) as usize].prev = index + 1;
        }
        self.partial[cls as usize] = index + 1;
    }

    fn partial_remove(&mut self, cls: u8, index: u32) {
        let (prev, next) = (self.pages[index as usize].prev, self.pages[index as usize].next);
        if prev != NONE {
            self.pages[(prev - 1) as usize].next = next
        } else {
            self.partial[cls as usize] = next
        }
        if next != NONE {
            self.pages[(next - 1) as usize].prev = prev;
        }
        self.pages[index as usize].prev = NONE;
        self.pages[index as usize].next = NONE;
    }

    fn zero(&mut self, at: usize, len: usize) {
        self.mem[at..at + len].fill(0);
    }

    // ---- Offset32 ---------------------------------------------------------
    pub fn allocate(&mut self, bytes: usize, align: usize) -> Result<Ref32, SlabError> {
        if self.retired {
            return Err(SlabError::Retired);
        }
        if bytes == 0 {
            return Err(SlabError::InvalidSize);
        }
        if align == 0 || !align.is_power_of_two() || align > MAX_ALIGNMENT {
            return Err(SlabError::InvalidAlignment);
        }
        let align = align.max(8);
        let cls = match if bytes <= MAX_SMALL { class_for(bytes, align) } else { None } {
            Some(c) => c,
            None => return self.allocate_large(bytes),
        };
        let index = if self.partial[cls as usize] != NONE {
            self.partial[cls as usize] - 1
        } else {
            let index = self.take_pages(1)?;
            self.pages[index as usize] = Page {
                kind: Kind::Small,
                cls,
                ..Page::EMPTY
            };
            self.partial_push(cls, index);
            self.stats.metadata_bytes += HEADER_BYTES;
            self.stats.live_pages += 1;
            index
        };
        let size = CLASS_BYTES[cls as usize] as usize;
        let page_base = index as usize * PAGE_BYTES + HEADER_BYTES;
        let page = self.pages[index as usize];
        let block = if page.free_head != 0 {
            let block = (page.free_head - 1) as usize;
            let at = page_base + block * size;
            let next = u16::from_le_bytes([self.mem[at], self.mem[at + 1]]);
            self.pages[index as usize].free_head = next;
            block
        } else {
            self.pages[index as usize].bump += 1;
            page.bump as usize
        };
        let p = &mut self.pages[index as usize];
        p.set_bit(block, true);
        p.live += 1;
        if p.free_head == 0 && p.bump == blocks_in_page(size) {
            self.partial_remove(cls, index);
        }
        let offset = page_base + block * size;
        self.zero(offset, size);
        self.stats.live_block_bytes += size;
        self.stats.live_objects += 1;
        Ref32::from_offset(offset).ok_or(SlabError::InvalidRef)
    }

    fn allocate_large(&mut self, bytes: usize) -> Result<Ref32, SlabError> {
        if bytes > self.reserve {
            return Err(SlabError::OutOfMemory);
        }
        let count = (bytes + HEADER_BYTES).div_ceil(PAGE_BYTES) as u32;
        let first = self.take_pages(count)?;
        self.pages[first as usize] = Page {
            kind: Kind::Large,
            run: count,
            ..Page::EMPTY
        };
        for p in 1..count {
            self.pages[(first + p) as usize].kind = Kind::LargeTail;
        }
        let payload = count as usize * PAGE_BYTES - HEADER_BYTES;
        let offset = first as usize * PAGE_BYTES + HEADER_BYTES;
        self.zero(offset, payload);
        self.stats.metadata_bytes += HEADER_BYTES;
        self.stats.live_pages += count as usize;
        self.stats.live_block_bytes += payload;
        self.stats.live_objects += 1;
        Ref32::from_offset(offset).ok_or(SlabError::InvalidRef)
    }

    /// (page, block or run length, usable bytes, large)
    fn locate(&self, r: Ref32) -> Result<(u32, usize, usize, bool), SlabError> {
        if self.retired {
            return Err(SlabError::Retired);
        }
        let offset = r.offset();
        if offset >= self.reserve as u64 {
            return Err(SlabError::InvalidRef);
        }
        let offset = offset as usize;
        let index = (offset / PAGE_BYTES) as u32;
        let within = offset % PAGE_BYTES;
        if index >= self.high_water || within < HEADER_BYTES {
            return Err(SlabError::InvalidRef);
        }
        let page = &self.pages[index as usize];
        match page.kind {
            Kind::Small => {
                let size = CLASS_BYTES[page.cls as usize] as usize;
                if !(within - HEADER_BYTES).is_multiple_of(size) {
                    return Err(SlabError::InvalidRef);
                }
                let block = (within - HEADER_BYTES) / size;
                if block >= page.bump as usize || !page.bit(block) {
                    return Err(SlabError::InvalidRef);
                }
                Ok((index, block, size, false))
            }
            // Meta (the handle table) is never an object, even at +64.
            Kind::Large if within == HEADER_BYTES => Ok((index, page.run as usize, page.run as usize * PAGE_BYTES - HEADER_BYTES, true)),
            _ => Err(SlabError::InvalidRef),
        }
    }

    pub fn free(&mut self, r: Ref32) -> Result<(), SlabError> {
        let (index, block, bytes, large) = self.locate(r)?;
        if large {
            let run = block as u32;
            self.zero(index as usize * PAGE_BYTES + HEADER_BYTES, bytes);
            self.push_free_run(index, run);
            self.stats.free_pages += run as usize;
            self.stats.live_pages -= run as usize;
            self.stats.metadata_bytes -= HEADER_BYTES;
            self.stats.live_block_bytes -= bytes;
            self.stats.live_objects -= 1;
            return Ok(());
        }
        let size = bytes;
        let at = index as usize * PAGE_BYTES + HEADER_BYTES + block * size;
        self.zero(at, size);
        let page = &mut self.pages[index as usize];
        let was_full = page.free_head == 0 && page.bump == blocks_in_page(size);
        page.set_bit(block, false);
        page.live -= 1;
        let (live, cls, next) = (page.live, page.cls, page.free_head);
        self.stats.live_block_bytes -= size;
        self.stats.live_objects -= 1;
        if live == 0 {
            if !was_full {
                self.partial_remove(cls, index);
            }
            self.push_free_run(index, 1);
            self.stats.free_pages += 1;
            self.stats.live_pages -= 1;
            self.stats.metadata_bytes -= HEADER_BYTES;
            return Ok(());
        }
        self.mem[at..at + 2].copy_from_slice(&next.to_le_bytes());
        self.pages[index as usize].free_head = (block + 1) as u16;
        if was_full {
            self.partial_push(cls, index);
        }
        Ok(())
    }

    /// Checked view: `len` must fit inside the live block.
    pub fn bytes(&self, r: Ref32, len: usize) -> Result<&[u8], SlabError> {
        let (_, _, usable, _) = self.locate(r)?;
        if len > usable {
            return Err(SlabError::InvalidSize);
        }
        let at = r.offset() as usize;
        Ok(&self.mem[at..at + len])
    }
    pub fn bytes_mut(&mut self, r: Ref32, len: usize) -> Result<&mut [u8], SlabError> {
        let (_, _, usable, _) = self.locate(r)?;
        if len > usable {
            return Err(SlabError::InvalidSize);
        }
        let at = r.offset() as usize;
        Ok(&mut self.mem[at..at + len])
    }

    // ---- Handle32 ---------------------------------------------------------
    fn entry(&self, slot: u32) -> (u32, u32) {
        let at = self.handle_base + slot as usize * 8;
        let m = &self.mem;
        (
            u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]]),
            u32::from_le_bytes([m[at + 4], m[at + 5], m[at + 6], m[at + 7]]),
        )
    }
    fn write_entry(&mut self, slot: u32, word: u32, gen: u32) {
        let at = self.handle_base + slot as usize * 8;
        self.mem[at..at + 4].copy_from_slice(&word.to_le_bytes());
        self.mem[at + 4..at + 8].copy_from_slice(&gen.to_le_bytes());
    }

    pub fn allocate_handle(&mut self, bytes: usize, align: usize) -> Result<Handle32, SlabError> {
        if self.retired {
            return Err(SlabError::Retired);
        }
        if self.handle_capacity == 0 || self.free_slot_head == NONE {
            return Err(SlabError::HandleTableFull);
        }
        let obj = self.allocate(bytes, align)?;
        let slot = self.free_slot_head - 1;
        let (next, gen) = self.entry(slot);
        self.free_slot_head = next;
        if next == NONE {
            self.free_slot_tail = NONE;
        }
        self.write_entry(slot, obj.word(), gen);
        Ok(Handle32((gen << self.slot_bits) | (slot + 1)))
    }

    pub fn resolve_handle(&self, h: Handle32) -> Result<Ref32, SlabError> {
        if self.retired {
            return Err(SlabError::Retired);
        }
        let slot1 = h.0 & ((1u32 << self.slot_bits) - 1);
        if self.handle_capacity == 0 || slot1 == 0 || slot1 > self.handle_capacity {
            return Err(SlabError::InvalidRef);
        }
        let (word, gen) = self.entry(slot1 - 1);
        if gen == 0 || (h.0 >> self.slot_bits) != gen {
            return Err(SlabError::StaleHandle);
        }
        Ref32::from_word(word).ok_or(SlabError::StaleHandle)
    }

    pub fn free_handle(&mut self, h: Handle32) -> Result<(), SlabError> {
        let r = self.resolve_handle(h)?;
        self.free(r)?;
        let slot = (h.0 & ((1u32 << self.slot_bits) - 1)) - 1;
        let (_, gen) = self.entry(slot);
        if gen == self.max_gen {
            self.write_entry(slot, NONE, 0); // quarantine: never wrap, never reuse
            self.quarantined += 1;
            self.stats.quarantined_handles = self.quarantined;
            return Ok(());
        }
        self.write_entry(slot, NONE, gen + 1);
        if self.free_slot_tail != NONE {
            let tail = self.free_slot_tail - 1;
            let (_, tail_gen) = self.entry(tail);
            self.write_entry(tail, slot + 1, tail_gen);
        } else {
            self.free_slot_head = slot + 1;
        }
        self.free_slot_tail = slot + 1;
        Ok(())
    }

    /// Actor STOP: drop everything. Idempotent.
    pub fn retire(&mut self) {
        self.mem = Vec::new();
        self.pages = Vec::new();
        self.retired = true;
        self.stats = Stats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heap(reserve: usize, budget: usize) -> ActorSlabHeap {
        ActorSlabHeap::new(Config::new(reserve, budget)).expect("config")
    }

    #[test]
    fn words_are_four_bytes_on_every_host() {
        assert_eq!(core::mem::size_of::<Ref32>(), 4);
        assert_eq!(core::mem::size_of::<Option<Ref32>>(), 4);
        assert_eq!(core::mem::size_of::<Handle32>(), 4);
    }

    #[test]
    fn allocate_zeroed_resolve_and_double_free() {
        let mut h = heap(1 << 20, 1 << 20);
        assert_eq!(h.stats().committed_bytes, 0);
        let a = h.allocate(24, 8).unwrap();
        assert!(h.bytes(a, 24).unwrap().iter().all(|&b| b == 0));
        h.bytes_mut(a, 24).unwrap().fill(0xAB);
        assert!(h.bytes(a, 32).is_ok());
        assert_eq!(h.bytes(a, 33), Err(SlabError::InvalidSize));
        h.free(a).unwrap();
        assert_eq!(h.free(a), Err(SlabError::InvalidRef));
        assert_eq!(h.allocate(0, 8), Err(SlabError::InvalidSize));
        assert_eq!(h.allocate(8, 3), Err(SlabError::InvalidAlignment));
        assert_eq!(h.allocate(8, 128), Err(SlabError::InvalidAlignment));
        for align in [8usize, 16, 32, 64] {
            for size in [1usize, 17, 50, 100, 700, 2048] {
                let r = h.allocate(size, align).unwrap();
                assert_eq!(r.offset() % align as u64, 0);
                h.free(r).unwrap();
            }
        }
        assert_eq!(h.stats().live_objects, 0);
    }

    #[test]
    fn packing_and_cross_class_page_reuse() {
        let mut h = heap(1 << 20, 1 << 20);
        let refs: Vec<_> = (0..253).map(|_| h.allocate(16, 8).unwrap()).collect();
        assert_eq!(h.stats().live_pages, 2); // 252 per page
        let committed = h.stats().committed_bytes;
        for r in &refs {
            h.free(*r).unwrap();
        }
        assert_eq!((h.stats().live_pages, h.stats().free_pages), (0, 2));
        let big: Vec<_> = (0..4).map(|_| h.allocate(1000, 8).unwrap()).collect();
        assert_eq!(h.stats().committed_bytes, committed);
        for r in big {
            h.free(r).unwrap();
        }
    }

    #[test]
    fn forged_interior_and_tail_page_refs_fail_closed() {
        let mut h = heap(1 << 20, 1 << 20);
        let a = h.allocate(32, 8).unwrap();
        for w in [a.word() + 1, 1, u32::MAX, a.word() + 512] {
            assert_eq!(h.free(Ref32::from_word(w).unwrap()), Err(SlabError::InvalidRef), "word {w}");
        }
        let big = h.allocate(3 * 4096, 8).unwrap();
        h.bytes_mut(big, 3 * 4096).unwrap().fill(0x01);
        let tail = Ref32::from_word(big.word() + (4096 / 8) as u32).unwrap();
        assert_eq!(h.free(tail), Err(SlabError::InvalidRef));
        h.free(big).unwrap();
        h.free(a).unwrap();
    }

    #[test]
    fn budget_failure_is_atomic() {
        let mut h = heap(1 << 20, 64 * 1024);
        let mut refs = Vec::new();
        let err = loop {
            match h.allocate(512, 8) {
                Ok(r) => refs.push(r),
                Err(e) => break e,
            }
        };
        assert_eq!(err, SlabError::BudgetExceeded);
        let before = h.stats();
        assert_eq!(h.allocate(100_000, 8), Err(SlabError::BudgetExceeded));
        assert_eq!(h.stats(), before);
        h.free(*refs.last().unwrap()).unwrap();
        assert!(h.allocate(512, 8).is_ok());
    }

    #[test]
    fn handles_fifo_stale_quarantine() {
        let mut c = Config::new(1 << 20, 1 << 20);
        c.handle_capacity = 4;
        let mut h = ActorSlabHeap::new(c).unwrap();
        assert_eq!((h.handle_slot_bits(), h.handle_max_generation()), (3, (1 << 29) - 1));
        let slot = |x: Handle32| x.0 & 7;
        let a = h.allocate_handle(40, 8).unwrap();
        let b = h.allocate_handle(40, 8).unwrap();
        assert_eq!((slot(a), slot(b)), (1, 2));
        h.free_handle(a).unwrap();
        assert_eq!(h.resolve_handle(a), Err(SlabError::StaleHandle));
        let (c1, d, e) = (
            h.allocate_handle(40, 8).unwrap(),
            h.allocate_handle(40, 8).unwrap(),
            h.allocate_handle(40, 8).unwrap(),
        );
        assert_eq!((slot(c1), slot(d), slot(e)), (3, 4, 1));
        assert_ne!(e, a);
        assert_eq!(h.allocate_handle(40, 8), Err(SlabError::HandleTableFull));

        let mut c = Config::new(1 << 20, 1 << 20);
        c.handle_capacity = 2;
        c.handle_generation_bits = 2;
        let mut small = ActorSlabHeap::new(c).unwrap();
        let mut n = 0;
        while let Ok(x) = small.allocate_handle(16, 8) {
            n += 1;
            small.free_handle(x).unwrap();
        }
        assert_eq!((n, small.stats().quarantined_handles), (6, 2));
        assert!(small.allocate(16, 8).is_ok());
    }

    #[test]
    fn stress_integrity() {
        let mut h = heap(8 << 20, 8 << 20);
        let mut live: Vec<(Ref32, usize, u8)> = Vec::new();
        let mut lcg: u32 = 12345;
        let mut next = || {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            lcg >> 8
        };
        for _ in 0..100_000 {
            if live.is_empty() || next() % 100 < 55 {
                let bytes = if next() % 100 < 95 {
                    1 + next() as usize % 256
                } else {
                    2049 + next() as usize % 9000
                };
                if let Ok(r) = h.allocate(bytes, 8) {
                    let tag = next() as u8;
                    h.bytes_mut(r, bytes).unwrap().fill(tag);
                    live.push((r, bytes, tag));
                }
            } else {
                let i = next() as usize % live.len();
                let (r, bytes, tag) = live.swap_remove(i);
                assert!(h.bytes(r, bytes).unwrap().iter().all(|&b| b == tag));
                h.free(r).unwrap();
            }
        }
        for (r, _, _) in live {
            h.free(r).unwrap();
        }
        assert_eq!((h.stats().live_objects, h.stats().live_pages), (0, 0));
    }

    #[test]
    fn retire_is_terminal() {
        let mut h = heap(1 << 20, 1 << 20);
        let r = h.allocate(100, 8).unwrap();
        h.retire();
        assert_eq!(h.allocate(16, 8), Err(SlabError::Retired));
        assert_eq!(h.free(r), Err(SlabError::Retired));
        assert_eq!(h.stats().committed_bytes, 0);
    }

    #[test]
    fn golden_matches_cpp_heap() {
        let expected: Vec<u32> = include_str!("../tests/fixtures/actor_slab_heap_golden_v1.txt")
            .split_whitespace()
            .map(|w| w.parse().unwrap())
            .collect();
        assert_eq!(
            super::conformance::golden_script_v1(),
            expected,
            "Rust and C++ slab heaps disagree on ref32 words"
        );
    }
}

/// Cross-backend conformance: every backend's slab heap must turn this exact
/// op script into exactly these words (fixture
/// `tests/fixtures/actor_slab_heap_golden_v1.txt`, generated by the C++ heap).
pub mod conformance {
    use super::*;

    /// Words per op: allocate -> ref32 word; free -> 0;
    /// allocate_handle -> handle word then its resolved ref32 word; free_handle -> 1.
    pub fn golden_script_v1() -> Vec<u32> {
        let mut c = Config::new(16 << 20, 16 << 20);
        c.handle_capacity = 64;
        let mut h = ActorSlabHeap::new(c).expect("golden config");
        let mut out = Vec::new();
        let mut live: Vec<Ref32> = Vec::new();
        let mut handles: Vec<Handle32> = Vec::new();
        let mut lcg: u32 = 7;
        let mut next = || {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            lcg >> 8
        };
        let sizes = [8usize, 16, 24, 40, 64, 100, 200, 512, 1000, 2048, 3000, 9000];
        let aligns = [8usize, 16, 32, 64];
        for _ in 0..2000 {
            match next() % 10 {
                0..=4 => {
                    let size = sizes[next() as usize % sizes.len()];
                    let align = aligns[next() as usize % aligns.len()];
                    let r = h.allocate(size, align).expect("golden allocate");
                    out.push(r.word());
                    live.push(r);
                }
                5..=6 if !live.is_empty() => {
                    let i = next() as usize % live.len();
                    h.free(live.swap_remove(i)).expect("golden free");
                    out.push(0);
                }
                7 => {
                    if let Ok(x) = h.allocate_handle(sizes[next() as usize % 6], 8) {
                        out.push(x.0);
                        out.push(h.resolve_handle(x).expect("golden resolve").word());
                        handles.push(x);
                    }
                }
                8 if !handles.is_empty() => {
                    let i = next() as usize % handles.len();
                    h.free_handle(handles.swap_remove(i)).expect("golden free_handle");
                    out.push(1);
                }
                _ => {}
            }
        }
        out
    }

    /// Canonical status codes shared by every backend (golden v2).
    pub fn code(e: SlabError) -> u32 {
        match e {
            SlabError::OutOfMemory => 1,
            SlabError::BudgetExceeded => 2,
            SlabError::InvalidSize => 3,
            SlabError::InvalidAlignment => 4,
            SlabError::InvalidRef => 5,
            SlabError::StaleHandle => 6,
            SlabError::HandleTableFull => 7,
            SlabError::Retired => 8,
            SlabError::InvalidConfig => 99,
        }
    }

    fn fail(e: SlabError) -> u32 {
        0x8000_0000 | code(e)
    }

    fn remember(v: &mut Vec<u32>, w: u32) {
        v.push(w);
        if v.len() > 8 {
            v.remove(0);
        }
    }

    /// Golden v2: failure paths and edge cases. Same script as the C++
    /// `tests/actor_slab_heap_golden_v2_test.cpp`; see that file for the format.
    pub fn golden_script_v2() -> Vec<u32> {
        let mut c = Config::new(256 * 1024, 128 * 1024);
        c.commit_granule = 16 * 1024;
        c.handle_capacity = 8;
        c.handle_generation_bits = 3;
        let mut h = ActorSlabHeap::new(c).expect("golden v2 config");
        let mut out = Vec::new();
        let mut lcg: u32 = 99;
        let mut next = || {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            lcg >> 8
        };
        let sizes = [
            0usize, 1, 8, 16, 17, 33, 48, 63, 64, 65, 100, 127, 128, 129, 255, 256, 257, 500, 1000, 2047, 2048, 2049,
        ];
        let aligns = [1usize, 8, 16, 32, 64, 128, 3];
        let larges = [3000usize, 4033, 4032, 8000, 12000, 20000];
        let (mut live, mut freed, mut handles, mut freed_handles) =
            (Vec::<u32>::new(), Vec::<u32>::new(), Vec::<u32>::new(), Vec::<u32>::new());
        let word = |w: u32| Ref32::from_word(w).expect("nonzero word");
        for _ in 0..3000 {
            let v = next() % 16;
            if v <= 5 {
                let s = sizes[next() as usize % 22];
                let a = aligns[next() as usize % 7];
                match h.allocate(s, a) {
                    Ok(r) => {
                        out.push(r.word());
                        live.push(r.word());
                    }
                    Err(e) => out.push(fail(e)),
                }
            } else if v <= 8 {
                if live.is_empty() {
                    continue;
                }
                let i = next() as usize % live.len();
                let w = live[i];
                out.push(h.free(word(w)).map_or_else(fail, |_| 0));
                live.swap_remove(i);
                remember(&mut freed, w);
            } else if v == 9 {
                if freed.is_empty() {
                    continue;
                }
                let w = freed[next() as usize % freed.len()];
                let r = h.free(word(w));
                out.push(r.map_or_else(fail, |_| 0));
                if r.is_ok() {
                    if let Some(i) = live.iter().position(|&l| l == w) {
                        live.swap_remove(i);
                    }
                }
            } else if v <= 11 {
                match h.allocate_handle(sizes[1 + next() as usize % 12], 8) {
                    Ok(x) => {
                        out.push(x.0);
                        handles.push(x.0);
                    }
                    Err(e) => out.push(fail(e)),
                }
            } else if v <= 13 {
                if handles.is_empty() {
                    continue;
                }
                let i = next() as usize % handles.len();
                let w = handles[i];
                out.push(h.free_handle(Handle32(w)).map_or_else(fail, |_| 0));
                handles.swap_remove(i);
                remember(&mut freed_handles, w);
            } else if v == 14 {
                if freed_handles.is_empty() {
                    continue;
                }
                let w = freed_handles[next() as usize % freed_handles.len()];
                out.push(h.resolve_handle(Handle32(w)).map_or_else(fail, |r| r.word()));
            } else {
                match h.allocate(larges[next() as usize % 6], 8) {
                    Ok(r) => {
                        out.push(r.word());
                        live.push(r.word());
                    }
                    Err(e) => out.push(fail(e)),
                }
            }
        }
        let st = h.stats();
        out.extend([
            st.live_objects as u32,
            st.live_pages as u32,
            st.free_pages as u32,
            st.quarantined_handles,
            st.committed_bytes as u32,
        ]);
        out
    }
}

#[cfg(test)]
mod golden_v2_tests {
    #[test]
    fn golden_v2_matches_cpp_heap() {
        let expected: Vec<u32> = include_str!("../tests/fixtures/actor_slab_heap_golden_v2.txt")
            .split_whitespace()
            .map(|w| w.parse().unwrap())
            .collect();
        let got = super::conformance::golden_script_v2();
        assert_eq!(got.len(), expected.len(), "golden v2 length");
        if let Some(i) = (0..got.len()).find(|&i| got[i] != expected[i]) {
            panic!("golden v2 word {i}: {} != {}", got[i], expected[i]);
        }
    }
}
