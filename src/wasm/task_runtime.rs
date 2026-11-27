//! Guest-side task runtime for the `atto.task.v1` ABI (docs/TASK_ABI_V1.md).
//!
//! The guest owns task *frames*; the host OresScheduler owns *scheduling*.
//! A frame is a fixed-size, generation-fenced arena block:
//!
//! ```text
//!   0 i32 task type      4 i32 resume PC      8 i32 state
//!  12 i32 generation    16 i64 result        24 i64 request
//!  32 i32 free link     40 .. 40+8*64 frame slots (params, then locals)
//! ```
//!
//! States: 1 LIVE (resumable), 2 RUNNING, 3 DONE, 5 CLOSED (on free list).
//! Handles are `generation << 32 | offset`; every export re-validates them,
//! so a stale, forged or recycled handle traps with panic code 7 instead of
//! reaching another task's frame.
//!
//! Each suspending callable compiles to `resume_k() -> i32 status`, reading
//! its frame from a module global set by `atto_task_resume`. Status values:
//! 0 DONE, 1 CALL, 2 AWAIT, 3 SPAWN, 4 DETACH, 5 SLEEP, 6 COOPERATE.
use super::*;
use crate::check::{TSuspend, TTask, TTaskStep};
use crate::check::MAX_TASK_SLOTS;

pub const PANIC_TASK: i32 = 7;
const SLOTS: u32 = 40;
pub(super) const TASK_FRAME_BYTES: i32 = 40 + 8 * MAX_TASK_SLOTS as i32;
const LIVE: i32 = 1;
const RUNNING: i32 = 2;
const DONE: i32 = 3;
const CLOSED: i32 = 5;
/// Back-edge budget per turn before a task cooperates (status 6).
const FUEL_PER_TURN: i32 = 4096;
/// Number of runtime functions appended by [`bodies`].
pub(super) const TASK_RUNTIME_FUNCS: u32 = 8;
pub(super) const RT_NEW: u32 = 0;
pub(super) const RT_CHECK: u32 = 1;
pub(super) const RT_HANDLE: u32 = 2;
pub(super) const RT_MAIN: u32 = 3;
pub(super) const RT_RESUME: u32 = 4;
pub(super) const RT_REQUEST: u32 = 5;
pub(super) const RT_RESULT: u32 = 6;
pub(super) const RT_CLOSE: u32 = 7;
pub(super) const RUNTIME_NAMES: [&str; 8] = [
    "atto.task_new", "atto.task_check", "atto.task_handle", "atto_task_main",
    "atto_task_resume", "atto_task_request", "atto_task_result", "atto_task_close",
];

fn task_panic(e: &mut Emit) {
    panic(&mut e.code, PANIC_TASK);
}

/// Typed slot load/store for a frame slot of value type `t`.
fn slot_load(e: &mut Emit, t: Type, offset: u32) {
    match vt(t).expect("value slot") {
        I32 => e.load_i32(offset),
        I64 => e.load_i64(offset),
        _ => { e.op(0x2B); uleb(&mut e.code, 3); uleb(&mut e.code, offset as u64); }
    }
}

fn slot_store(e: &mut Emit, t: Type, offset: u32) {
    match vt(t).expect("value slot") {
        I32 => e.store_i32(offset),
        I64 => e.store_i64(offset),
        _ => { e.op(0x39); uleb(&mut e.code, 3); uleb(&mut e.code, offset as u64); }
    }
}

fn rt(l: &Layout, which: u32) -> u32 {
    l.func(l.task_first_func + which)
}

/// Scratch locals of a resume body, after params, locals and ticket.
pub(super) struct ResumeLocals { pub fp: u32, pub pc: u32, pub fuel: u32, pub child: u32 }

/// Emit the body of `resume_k` for one planned task. `slots` are the frame
/// slot types (callable params then locals).
pub(super) fn resume_body(e: &mut Emit, p: &TProgram, task: &TTask, slots: &[Type], r: ResumeLocals) {
    let (frame, wake, _) = e.l.task_globals.expect("task globals");
    let save = |e: &mut Emit| {
        e.get(r.fp); e.get(r.pc); e.store_i32(4);
        for (i, t) in slots.iter().enumerate() {
            e.get(r.fp); e.get(i as u32); slot_store(e, *t, SLOTS + 8 * i as u32);
        }
    };
    e.global_get(frame); e.set(r.fp);
    e.get(r.fp); e.load_i32(4); e.set(r.pc);
    for (i, t) in slots.iter().enumerate() {
        e.get(r.fp); slot_load(e, *t, SLOTS + 8 * i as u32); e.set(i as u32);
    }
    e.i32c(FUEL_PER_TURN); e.set(r.fuel);
    e.open(0x03, 0x40, Frame::Task);
    for (pc, step) in task.steps.iter().enumerate() {
        e.get(r.pc); e.i32c(pc as i32); e.op(0x46);
        e.open(0x04, 0x40, Frame::Other);
        let next = |e: &mut Emit, to: u32| { e.i32c(to as i32); e.set(r.pc); e.br_to(Frame::Task); };
        match step {
            TTaskStep::Run(body) => {
                e.stmts(body);
                next(e, pc as u32 + 1);
            }
            TTaskStep::Guard { condition, on_false } => {
                e.expr(condition);
                e.open(0x04, 0x40, Frame::Other);
                e.i32c(pc as i32 + 1); e.set(r.pc);
                e.op(0x05);
                e.i32c(*on_false as i32); e.set(r.pc);
                e.close();
                e.br_to(Frame::Task);
            }
            TTaskStep::Jump { to, back } => {
                if *back {
                    e.get(r.fuel); e.i32c(1); e.op(0x6B); e.set(r.fuel);
                    e.get(r.fuel); e.op(0x45);
                    e.open(0x04, 0x40, Frame::Other);
                    e.i32c(*to as i32); e.set(r.pc);
                    save(e);
                    e.i32c(6); e.op(0x0F); // COOPERATE
                    e.close();
                }
                next(e, *to);
            }
            TTaskStep::Suspend(op) => {
                let status = match op {
                    TSuspend::Call { function, args } | TSuspend::Spawn { function, args, .. } => {
                        let child_type = p.tasks.iter().position(|t| t.function == *function)
                            .expect("planned callee task");
                        e.i32c(child_type as i32);
                        e.call(rt(e.l, RT_NEW));
                        e.set(r.child);
                        for (j, a) in args.iter().enumerate() {
                            e.get(r.child); e.expr(a); slot_store(e, a.ty, SLOTS + 8 * j as u32);
                        }
                        e.get(r.fp);
                        e.get(r.child); e.call(rt(e.l, RT_HANDLE));
                        e.store_i64(24);
                        match op {
                            TSuspend::Call { .. } => 1,
                            TSuspend::Spawn { detached: false, .. } => 3,
                            _ => 4,
                        }
                    }
                    TSuspend::Await(v) => { e.get(r.fp); e.expr(v); e.store_i64(24); 2 }
                    TSuspend::Sleep(v) => { e.get(r.fp); e.expr(v); e.store_i64(24); 5 }
                };
                e.i32c(pc as i32 + 1); e.set(r.pc);
                save(e);
                e.i32c(status); e.op(0x0F);
            }
            TTaskStep::Receive { dest, ty } => {
                e.global_get(wake);
                if vt(*ty) == Some(I32) { e.op(0xA7); } // i32.wrap_i64 (bool)
                e.set(*dest);
                next(e, pc as u32 + 1);
            }
            TTaskStep::Return(value) => {
                if let Some(v) = value {
                    e.get(r.fp);
                    e.expr(v);
                    if vt(v.ty) == Some(I32) { e.op(0xAD); } // i64.extend_i32_u
                    e.store_i64(16);
                }
                e.i32c(0); e.op(0x0F); // DONE
            }
            TTaskStep::Unreachable => task_panic(e),
        }
        e.close();
    }
    task_panic(e); // corrupt PC
    e.close();
    task_panic(e);
}

/// Runtime functions appended after every other function (see RT_*).
pub(super) fn bodies(p: &TProgram, l: &Layout) -> Vec<(Sig, Vec<u8>)> {
    let (_, _, free) = l.task_globals.expect("task globals");
    let (frame, wake, _) = l.task_globals.unwrap();
    let mut out = Vec::with_capacity(TASK_RUNTIME_FUNCS as usize);
    let make = |locals: &[Type]| Emit {
        l,
        code: locals_decl(locals),
        frames: Vec::new(),
        void_main: false,
        active_iterators: Vec::new(),
        state_ticket_slot: None,
        // Runtime ops never shift; the slot index is past every local.
        shift_slot: 2 + locals.len() as u32,
        shift_used: false,
        state_write_depth: 0,
        uw: None,
        in_recover: false,
        calls: Vec::new(),
        landings: Vec::new(),
    };
    let clear = |e: &mut Emit, ptr: u32| {
        e.get(ptr); e.i64c(0); e.store_i64(16);
        e.get(ptr); e.i64c(0); e.store_i64(24);
        for n in 0..MAX_TASK_SLOTS as u32 {
            e.get(ptr); e.i64c(0); e.store_i64(SLOTS + 8 * n);
        }
    };

    // RT_NEW(type: i32) -> i32 frame pointer. Locals 1=ptr, 2=generation.
    let mut e = make(&[Type::Bool, Type::Bool]);
    e.global_get(free); e.op(0x45);
    e.open(0x04, 0x40, Frame::Other);
    // 8-byte align the shared bump allocator, then take a fresh frame.
    e.global_get(1); e.i32c(7); e.op(0x6A); e.i32c(-8); e.op(0x71); e.global_set(1);
    e.i32c(TASK_FRAME_BYTES); e.call(l.helper(H_ALLOC)); e.set(1);
    e.i32c(1); e.set(2);
    e.op(0x05);
    e.global_get(free); e.set(1);
    e.get(1); e.load_i32(8); e.i32c(CLOSED); e.op(0x47);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1); e.load_i32(32); e.global_set(free);
    e.get(1); e.load_i32(12); e.set(2);
    e.get(2); e.i32c(-1); e.op(0x46);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close(); // no generation wrap
    e.get(2); e.i32c(1); e.op(0x6A); e.set(2);
    e.close();
    e.get(1); e.get(0); e.store_i32(0);
    e.get(1); e.i32c(0); e.store_i32(4);
    e.get(1); e.i32c(LIVE); e.store_i32(8);
    e.get(1); e.get(2); e.store_i32(12);
    e.get(1); e.i32c(0); e.store_i32(32);
    clear(&mut e, 1);
    e.get(1);
    e.op(0x0B);
    out.push((Sig(vec![I32], vec![I32]), e.code));

    // RT_CHECK(handle: i64) -> i32 validated frame pointer. 1=ptr, 2=gen.
    let mut e = make(&[Type::Bool, Type::Bool]);
    e.get(0); e.op(0xA7); e.set(1);
    e.get(1); e.i32c(8); e.op(0x49);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1); e.i32c(HEAP_PAGES as i32 * 65536 - TASK_FRAME_BYTES); e.op(0x4B);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1); e.i32c(7); e.op(0x71);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1); e.i32c(TASK_FRAME_BYTES); e.op(0x6A); e.global_get(1); e.op(0x4B);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(0); e.i64c(32); e.op(0x88); e.op(0xA7); e.set(2);
    e.get(2); e.op(0x45);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1); e.load_i32(12); e.get(2); e.op(0x47);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1);
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![I32]), e.code));

    // RT_HANDLE(ptr: i32) -> i64.
    let mut e = make(&[]);
    e.get(0); e.load_i32(12); e.op(0xAD); e.i64c(32); e.op(0x86);
    e.get(0); e.op(0xAD); e.op(0x84);
    e.op(0x0B);
    out.push((Sig(vec![I32], vec![I64]), e.code));

    // RT_MAIN() -> i64: the root task frame of main.
    let mut e = make(&[]);
    let main_type = p.tasks.iter().position(|t| t.function == p.main).expect("main task");
    e.i32c(main_type as i32); e.call(rt(l, RT_NEW)); e.call(rt(l, RT_HANDLE));
    e.op(0x0B);
    out.push((Sig(vec![], vec![I64]), e.code));

    // RT_RESUME(handle: i64, wake: i64) -> i32 status. 2=ptr, 3=status.
    let mut e = make(&[Type::Bool, Type::Bool]);
    e.get(0); e.call(rt(l, RT_CHECK)); e.set(2);
    e.get(2); e.load_i32(8); e.i32c(LIVE); e.op(0x47);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(2); e.i32c(RUNNING); e.store_i32(8);
    e.get(2); e.global_set(frame);
    e.get(1); e.global_set(wake);
    for (ty, task) in p.tasks.iter().enumerate() {
        e.get(2); e.load_i32(0); e.i32c(ty as i32); e.op(0x46);
        e.open(0x04, 0x40, Frame::Other);
        e.call(l.func(task.function)); e.set(3);
        e.get(2); e.i32c(DONE); e.i32c(LIVE); e.get(3); e.op(0x45); e.op(0x1B); e.store_i32(8);
        e.get(3); e.op(0x0F);
        e.close();
    }
    task_panic(&mut e); // unknown task type
    e.op(0x0B);
    out.push((Sig(vec![I64, I64], vec![I32]), e.code));

    // RT_REQUEST(handle) -> i64.
    let mut e = make(&[]);
    e.get(0); e.call(rt(l, RT_CHECK)); e.load_i64(24);
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![I64]), e.code));

    // RT_RESULT(handle) -> i64: only a DONE task has a result. 1=ptr.
    let mut e = make(&[Type::Bool]);
    e.get(0); e.call(rt(l, RT_CHECK)); e.set(1);
    e.get(1); e.load_i32(8); e.i32c(DONE); e.op(0x47);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    e.get(1); e.load_i64(16);
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![I64]), e.code));

    // RT_CLOSE(handle): LIVE/DONE -> CLOSED, cleared and recycled. 1=ptr.
    let mut e = make(&[Type::Bool]);
    e.get(0); e.call(rt(l, RT_CHECK)); e.set(1);
    e.get(1); e.load_i32(8); e.i32c(LIVE); e.op(0x46);
    e.get(1); e.load_i32(8); e.i32c(DONE); e.op(0x46);
    e.op(0x72); e.op(0x45);
    e.open(0x04, 0x40, Frame::Other); task_panic(&mut e); e.close();
    clear(&mut e, 1);
    e.get(1); e.i32c(0); e.store_i32(4);
    e.get(1); e.i32c(CLOSED); e.store_i32(8);
    e.get(1); e.global_get(free); e.store_i32(32);
    e.get(1); e.global_set(free);
    e.op(0x0B);
    out.push((Sig(vec![I64], vec![]), e.code));
    out
}
