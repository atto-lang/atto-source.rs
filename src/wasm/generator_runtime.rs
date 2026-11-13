//! Real guest-Wasm generator runtime code emission for the explicitly admitted
//! zero-capture, finite Generator<int> profile. Functions are ordinary Wasm
//! code with no host imports or JS generator shims.
use super::*;

/// Four guest functions after source callables: new, checked_ptr, next, close.
/// A frame is 32 bytes and always lives inside the instance-owned arena.
/// All loads from a handle go through the generation/bounds validator.
pub(super) fn bodies(p: &TProgram, l: &Layout) -> Vec<(Sig, Vec<u8>)> {
    assert!(p.heap && !p.generators.is_empty());
    let free = l.gen_free_global.expect("generator free-list global");
    let first = l.func(l.gen_first_func);
    let mut out = Vec::with_capacity(4);

    let make = |locals: &[Type]| Emit {
        l,
        code: locals_decl(locals),
        frames: Vec::new(),
        void_main: false,
        active_iterators: Vec::new(),
    };

    // new(type_id: i32) -> i64 generation-fenced handle.
    // Local 1=frame pointer, local 2=generation.
    let mut e = make(&[Type::Bool, Type::Bool]);
    e.global_get(free);
    e.op(0x45); // free list empty
    e.open(0x04, 0x40, Frame::Other);
    // Align the shared bump allocator to 8 bytes. Other allocation
    // paths (e.g. string concatenation) need not have done so.
    e.global_get(1);
    e.i32c(7);
    e.op(0x6A);
    e.i32c(-8);
    e.op(0x71);
    e.global_set(1);
    e.i32c(32);
    e.call(l.helper(H_ALLOC));
    e.set(1);
    e.i32c(1);
    e.set(2);
    e.op(0x05); // reuse a CLOSED 32-byte frame
    e.global_get(free);
    e.set(1);
    e.get(1);
    e.load_i32(24);
    e.global_set(free);
    e.get(1);
    e.load_i32(12);
    e.set(2);
    e.get(2);
    e.i32c(-1);
    e.op(0x46);
    e.open(0x04, 0x40, Frame::Other);
    e.invalid_gen(); // generation wrap/ABA prohibited
    e.close();
    e.get(2);
    e.i32c(1);
    e.op(0x6A);
    e.set(2);
    e.close();
    e.get(1); e.get(0); e.store_i32(0); // generator type
    e.get(1); e.i32c(0); e.store_i32(4); // PC = 0
    e.get(1); e.i32c(0); e.store_i32(8); // CREATED
    e.get(1); e.get(2); e.store_i32(12); // generation
    e.get(1); e.i64c(0); e.store_i64(16); // last value
    e.get(1); e.i32c(0); e.store_i32(24); // no active child/free link
    e.get(2); e.op(0xAD); // i64.extend_i32_u generation
    e.i64c(32); e.op(0x86); // shift left 32
    e.get(1); e.op(0xAD); e.op(0x84); // combine arena offset
    e.op(0x0B);
    out.push((Sig(vec![I32], vec![I64]), e.code));

    // checked_ptr(handle: i64) -> i32.
    let mut e = make(&[Type::Bool, Type::Bool]);
    e.get(0); e.op(0xA7); e.set(1); // i32.wrap_i64
    e.get(1); e.i32c(8); e.op(0x49); // ptr < 8
    e.open(0x04, 0x40, Frame::Other); e.invalid_gen(); e.close();
    e.get(1); e.i32c((HEAP_PAGES * 65536 - 32) as i32);
    e.op(0x4B); // ptr > last complete frame start
    e.open(0x04, 0x40, Frame::Other); e.invalid_gen(); e.close();
    e.get(1); e.i32c(7); e.op(0x71); // alignment
    e.open(0x04, 0x40, Frame::Other); e.invalid_gen(); e.close();
    e.get(0); e.i64c(32); e.op(0x88); e.op(0xA7); // generation
    e.set(2);
    e.get(2); e.op(0x45); // zero epoch invalid
    e.open(0x04, 0x40, Frame::Other); e.invalid_gen(); e.close();
    e.get(1); e.load_i32(12); e.get(2); e.op(0x47);
    e.open(0x04, 0x40, Frame::Other); e.invalid_gen(); e.close();
    e.get(1);
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![I32]), e.code));

    // next(handle: i64) -> i32: 1=YIELDED, 0=DONE.
    // Pull-only progress: one item per invocation, no eager collection.
    // Locals: 1=frame pointer, 2=PC, 3=generator type.
    let mut e = make(&[Type::Bool, Type::Bool, Type::Bool]);
    e.get(0); e.call(first + 1); e.set(1);
    e.get(1); e.load_i32(8); e.i32c(3); e.op(0x46);
    e.open(0x04, 0x40, Frame::Other);
    e.i32c(0); e.op(0x0F); // DONE stays DONE
    e.close();
    e.get(1); e.load_i32(8); e.i32c(0); e.op(0x46);
    e.get(1); e.load_i32(8); e.i32c(2); e.op(0x46);
    e.op(0x72); e.op(0x45); // only CREATED/SUSPENDED can run
    e.open(0x04, 0x40, Frame::Other); e.invalid_gen(); e.close();
    e.get(1); e.i32c(1); e.store_i32(8); // RUNNING
    e.get(1); e.load_i32(0); e.set(3);
    e.get(1); e.load_i32(4); e.set(2);
    for (gen_id, gen) in p.generators.iter().enumerate() {
        e.get(3); e.i32c(gen_id as i32); e.op(0x46);
        e.open(0x04, 0x40, Frame::Other);
        for (pc, item) in gen.items.iter().enumerate() {
            let crate::check::TExpr::I64(value) = &item.e else {
                unreachable!("admitted generator IR must be checked int literals");
            };
            e.get(2); e.i32c(pc as i32); e.op(0x46);
            e.open(0x04, 0x40, Frame::Other);
            e.get(1); e.i64c(*value); e.store_i64(16);
            e.get(1); e.i32c(pc as i32 + 1); e.store_i32(4);
            e.get(1); e.i32c(2); e.store_i32(8); // SUSPENDED
            e.i32c(1); e.op(0x0F);
            e.close();
        }
        e.get(2); e.i32c(gen.items.len() as i32); e.op(0x47);
        e.open(0x04, 0x40, Frame::Other);
        e.invalid_gen(); // corrupt PC is never silently DONE
        e.close();
        e.get(1); e.i32c(3); e.store_i32(8); // DONE
        e.i32c(0); e.op(0x0F);
        e.close();
    }
    e.invalid_gen(); // invalid generator type
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![I32]), e.code));

    // close(handle: i64) -> (): double close is idempotent until frame reuse.
    // Reuse bumps the generation before handing out a new live handle.
    let mut e = make(&[Type::Bool]);
    e.get(0); e.call(first + 1); e.set(1);
    e.get(1); e.load_i32(8); e.i32c(5); e.op(0x46);
    e.open(0x04, 0x40, Frame::Other);
    e.op(0x0F);
    e.close();
    e.get(1); e.load_i32(8); e.i32c(1); e.op(0x46);
    e.get(1); e.load_i32(8); e.i32c(4); e.op(0x46);
    e.op(0x72);
    e.open(0x04, 0x40, Frame::Other);
    e.invalid_gen(); // never recycle a RUNNING/CLOSING frame
    e.close();
    e.get(1); e.i32c(5); e.store_i32(8);
    e.get(1); e.global_get(free); e.store_i32(24);
    e.get(1); e.global_set(free);
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![]), e.code));
    out
}
