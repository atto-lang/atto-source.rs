//! Private guest-owned bounded closure arena for the first fnc(int) => int
//! profile. This is a constrained runtime, not the general callable ABI.
//! Handles are i32 {generation:23, slot:9}, never raw linear-memory pointers.
//! The only access to selector/capture fields is through checked_ptr.
//! Each slot has five i64 fields, an i32 generation, an i32 state/free link.
//! An active slot has state=-1; a closed slot links to the free head;
//! a retired slot has state=-2 and is never reused.
use super::*;

pub(super) const SLOTS: i32 = 256;
const FRAME_BYTES: i32 = 48;
const SLOT_MASK: i32 = 511;
const MAX_GENERATION: i32 = (1 << 23) - 1;
pub(super) const PANIC_CALLABLE: i32 = 6;

pub(super) fn bodies(l: &Layout) -> Vec<(Sig, Vec<u8>)> {
    let (base, next, free) = l.closure_globals.expect("generated closure arena globals");
    let first = l.func(l.closure_first_func);
    let create = || Emit {
        l, code: Vec::new(), frames: Vec::new(),
        void_main: false, active_iterators: Vec::new(),
    };
    let mut out = Vec::with_capacity(3);

    // create() -> i32: returns an opaque generation-fenced handle.
    // Additional locals: 0=slot, 1=ptr, 2=generation.
    let mut e = create();
    e.code = locals_decl(&[Type::Bool, Type::Bool, Type::Bool]);
    e.global_get(base);
    e.op(0x45); // base == 0: reserve fixed arena once per Wasm instance
    e.open(0x04, 0x40, Frame::Other);
    // Shared heap allocations may not preserve 8-byte alignment.
    e.global_get(1);
    e.i32c(7);
    e.op(0x6A);
    e.i32c(-8);
    e.op(0x71);
    e.global_set(1);
    e.i32c(SLOTS * FRAME_BYTES);
    e.call(l.helper(H_ALLOC));
    e.global_set(base);
    e.close();

    e.global_get(free);
    e.op(0x45); // empty free list -> fresh slot
    e.open(0x04, 0x40, Frame::Other);
    e.global_get(next);
    e.set(0);
    e.get(0);
    e.i32c(SLOTS);
    e.op(0x4B); // unsigned slot > capacity
    e.open(0x04, 0x40, Frame::Other);
    panic(&mut e.code, PANIC_HEAP);
    e.close();
    e.get(0); e.i32c(1); e.op(0x6A); e.global_set(next);
    // ptr = base + (slot - 1)*48
    e.global_get(base);
    e.get(0); e.i32c(1); e.op(0x6B);
    e.i32c(FRAME_BYTES); e.op(0x6C); e.op(0x6A);
    e.set(1);
    e.i32c(1); e.set(2);
    e.op(0x05);
    // Reuse a slot that was closed and generation-incremented.
    e.global_get(free); e.set(0);
    e.get(0); e.i32c(1); e.op(0x49);
    e.get(0); e.global_get(next); e.op(0x4F); // slot >= next
    e.op(0x72);
    e.open(0x04, 0x40, Frame::Other);
    panic(&mut e.code, PANIC_CALLABLE);
    e.close();
    e.global_get(base);
    e.get(0); e.i32c(1); e.op(0x6B);
    e.i32c(FRAME_BYTES); e.op(0x6C); e.op(0x6A);
    e.set(1);
    e.get(1); e.load_i32(44); e.global_set(free);
    e.get(1); e.load_i32(40); e.set(2);
    e.get(2); e.op(0x45); // generation must never be zero
    e.get(2); e.i32c(MAX_GENERATION); e.op(0x4B); // > maximum
    e.op(0x72);
    e.open(0x04, 0x40, Frame::Other);
    panic(&mut e.code, PANIC_CALLABLE);
    e.close();
    e.close();
    e.get(1); e.get(2); e.store_i32(40);
    e.get(1); e.i32c(-1); e.store_i32(44); // live
    // Clear record body before any capture writes; do not leak old captures.
    for at in [0, 8, 16, 24, 32] {
        e.get(1); e.i64c(0); e.store_i64(at);
    }
    e.get(2); e.i32c(9); e.op(0x74); // generation << 9
    e.get(0); e.op(0x72); // | one-based slot
    e.op(0x0B);
    out.push((Sig(vec![], vec![I32]), e.code));

    // checked_ptr(handle: i32) -> i32. Validate every field read/write,
    // including stale, retired, zero, out-of-range, and closed handles.
    // Locals 1=slot, 2=generation, 3=ptr.
    let mut e = create();
    e.code = locals_decl(&[Type::Bool, Type::Bool, Type::Bool]);
    e.get(0); e.i32c(SLOT_MASK); e.op(0x71); e.set(1);
    e.get(0); e.i32c(9); e.op(0x76); e.set(2); // shr_u
    e.get(1); e.op(0x45);
    e.get(1); e.i32c(SLOTS); e.op(0x4B);
    e.op(0x72);
    e.get(1); e.global_get(next); e.op(0x4F);
    e.op(0x72);
    e.get(2); e.op(0x45); e.op(0x72);
    e.global_get(base); e.op(0x45); e.op(0x72);
    e.open(0x04, 0x40, Frame::Other);
    panic(&mut e.code, PANIC_CALLABLE);
    e.close();
    e.global_get(base);
    e.get(1); e.i32c(1); e.op(0x6B);
    e.i32c(FRAME_BYTES); e.op(0x6C); e.op(0x6A);
    e.set(3);
    e.get(3); e.load_i32(40); e.get(2); e.op(0x47); // gen mismatch
    e.get(3); e.load_i32(44); e.i32c(-1); e.op(0x47); // not live
    e.op(0x72);
    e.open(0x04, 0x40, Frame::Other);
    panic(&mut e.code, PANIC_CALLABLE);
    e.close();
    e.get(3); e.op(0x0B);
    out.push((Sig(vec![I32], vec![I32]), e.code));

    // close(handle: i32) -> void. Scrub all captures and retire the handle.
    // A stale or double close traps before changing memory/free-list state.
    // Locals 1=checked_ptr, 2=generation.
    let mut e = create();
    e.code = locals_decl(&[Type::Bool, Type::Bool]);
    e.get(0); e.call(first + 1); e.set(1);
    for at in [0, 8, 16, 24, 32] {
        e.get(1); e.i64c(0); e.store_i64(at);
    }
    e.get(1); e.load_i32(40); e.set(2);
    e.get(2); e.i32c(MAX_GENERATION); e.op(0x46);
    e.open(0x04, 0x40, Frame::Other);
    e.get(1); e.i32c(-2); e.store_i32(44); // retired: never recyclable
    e.op(0x05);
    e.get(1); e.get(2); e.i32c(1); e.op(0x6A); e.store_i32(40);
    e.get(1); e.global_get(free); e.store_i32(44);
    e.get(0); e.i32c(SLOT_MASK); e.op(0x71); e.global_set(free);
    e.close();
    e.op(0x0B);
    out.push((Sig(vec![I32], vec![]), e.code));

    out
}
