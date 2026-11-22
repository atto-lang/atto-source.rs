//! Experimental portable-v5 external-pending scalar future task arena.
//! Completely AOT-generated instructions, private one-page Wasm memory.
//! No JSPI/Promise stack suspension, dynamic interpreter or guest imports.
//! Separate opt-in compilation entrypoint; ordinary v3 bytes remain unchanged.
//! Each await operand is a host-authorized external future key (i64). Await does not auto-complete.
use super::{name,sleb,uleb,I32,I64,Sig,TProgram,Types};
use super::portable_task_v3::{PortableTaskV3,ScalarOperand,Step};

const SLOTS:usize=16;
const FRAME:u32=96;
const OWNER:u32=0;
const EPOCH:u32=4;
const GEN:u32=8;
const STATE:u32=12;
const RESULT:u32=16;
const AWAITED:u32=24;
const LOCAL_BASE:u32=32;
// The 27-bit generation never wraps. Final positive handle fits signed i32.
const MAX_GEN:i32=(1<<27)-2;
const INVALID:i32=-1;
const FULL:i32=-2;
const READY:i32=0;
const PENDING:i32=1;
const DONE:i32=2;
const CANCELLED:i32=4;
// Waiting states: 1..=N. Ready states: N+4..=2N+3.
// A distinct ready state means a task cannot resume before an authorized complete.

pub(super) struct PortableTaskV5 { steps:Vec<Step>, locals:usize }
impl PortableTaskV5 {
    pub(super) fn from_program(p:&TProgram)->Option<Self> {
        let checked=PortableTaskV3::from_program(p)?;
        Some(Self{steps:checked.steps,locals:checked.locals})
    }
    pub(super) fn append_functions(&self,types:&mut Types,functions:&mut Vec<u32>,bodies:&mut Vec<Vec<u8>>) {
        // Start returns an opaque positive i32 handle; all subsequent
        // operations require handle + owner + epoch and reject mismatches.
        let signatures=[
            Sig(vec![I32,I32],vec![I32]),
            Sig(vec![I32,I32,I32],vec![I32]),
            Sig(vec![I32,I32,I32],vec![I32]),
            Sig(vec![I32,I32,I32],vec![I32]),
            Sig(vec![I32,I32,I32],vec![I64]),
            Sig(vec![],vec![I32]),
            Sig(vec![I32,I32,I32,I64,I64],vec![I32]),
            Sig(vec![I32,I32,I32],vec![I64]),
        ];
        let bodies_v5=[self.start(),self.resume(),self.cancel(),self.drop_frame(),self.result(),self.capacity(),self.complete(),self.pending()];
        for (sig,body) in signatures.into_iter().zip(bodies_v5) {
            functions.push(types.idx(sig));
            bodies.push(body);
        }
    }
    pub(super) fn append_exports(&self,ex:&mut Vec<u8>,first:u32) {
        for (i,name_) in ["task_start_v5","task_resume_v5","task_cancel_v5",
            "task_drop_v5","task_result_v5","task_capacity_v5",
            "task_complete_v5","task_pending_v5"].into_iter().enumerate() {
            name(ex,name_);ex.push(0);uleb(ex,first as u64+i as u64);
        }
    }
    pub(super) fn append_names(&self,names:&mut Vec<String>) {
        names.extend(["atto.task_start_v5","atto.task_resume_v5","atto.task_cancel_v5",
            "atto.task_drop_v5","atto.task_result_v5","atto.task_capacity_v5",
            "atto.task_complete_v5","atto.task_pending_v5"].into_iter().map(str::to_owned));
    }
    // Slot+1 occupies the low four bits. The high bits are a persistent
    // non-zero per-slot generation, fenced against integer wraparound.
    fn checked_frame(c:&mut Vec<u8>,trap:bool) {
        Self::checked_frame_at(c,trap,3,4);
    }
    fn checked_frame_at(c:&mut Vec<u8>,trap:bool,ptr:u32,generation:u32) {
        // params: handle(0), owner(1), epoch(2).
        // ptr and generation are caller-selected i32 local indices.
        lget(c,0);i32c(c,SLOTS as i32);c.push(0x4C);bad_if(c,trap); // handle <= 16
        for param in [1,2] {
            lget(c,param);i32c(c,0);c.push(0x4C);bad_if(c,trap);
        }
        lget(c,0);i32c(c,1);c.push(0x6B);
        i32c(c,(SLOTS-1) as i32);c.push(0x71);
        i32c(c,FRAME as i32);c.push(0x6C);lset(c,ptr);
        lget(c,0);i32c(c,1);c.push(0x6B);
        i32c(c,4);c.push(0x76);lset(c,generation); // logical shift
        lget(c,generation);i32c(c,0);c.push(0x4C);bad_if(c,trap);
        load32(c,ptr,GEN);lget(c,generation);c.push(0x47);bad_if(c,trap);
        load32(c,ptr,STATE);c.push(0x45);bad_if(c,trap);
        load32(c,ptr,OWNER);lget(c,1);c.push(0x47);bad_if(c,trap);
        load32(c,ptr,EPOCH);lget(c,2);c.push(0x47);bad_if(c,trap);
    }
    fn clear_values(&self,c:&mut Vec<u8>,ptr:u32) {
        for off in [RESULT,AWAITED] {
            store64_zero(c,ptr,off);
        }
        for i in 0..self.locals {
            store64_zero(c,ptr,LOCAL_BASE+8*i as u32);
        }
    }
    fn emit_operand(c:&mut Vec<u8>,ptr:u32,value:ScalarOperand) {
        match value {
            ScalarOperand::Literal(n)=>i64c(c,n),
            ScalarOperand::Local(slot)=>load64(c,ptr,LOCAL_BASE+8*slot),
        }
    }
    fn emit_step(&self,c:&mut Vec<u8>,ptr:u32,i:usize) {
        let step=&self.steps[i];
        for (slot,val) in &step.assignments {
            lget(c,ptr);Self::emit_operand(c,ptr,*val);
            c.push(0x37);memarg(c,3,LOCAL_BASE+8*slot);
        }
        // Capture await operand before yielding. Each subsequent resume
        // executes only one statically generated continuation segment.
        lget(c,ptr);Self::emit_operand(c,ptr,step.await_operand);
        c.push(0x37);memarg(c,3,AWAITED);
    }
    fn start(&self)->Vec<u8> {
        // Three i32 locals: frame pointer=2, generation=3, reserved=4.
        let mut c=vec![1,3,I32];
        for param in [0,1] {
            lget(&mut c,param);i32c(&mut c,0);c.push(0x4C);bad_if(&mut c,false);
        }
        // Explicitly unroll a 16-slot allocation check. No runtime loop or
        // guest compiler/interpreter is shipped into the execution module.
        for slot in 0..SLOTS {
            i32c(&mut c,(slot as u32*FRAME) as i32);lset(&mut c,2);
            load32(&mut c,2,STATE);c.push(0x45); // state == 0 (free)
            load32(&mut c,2,GEN);i32c(&mut c,MAX_GEN);c.push(0x48); // gen < limit
            c.push(0x71); // both checks
            c.extend_from_slice(&[0x04,0x40]); // if
            load32(&mut c,2,GEN);i32c(&mut c,1);c.push(0x6A);lset(&mut c,3);
            store32_local(&mut c,2,GEN,3);
            store32_local(&mut c,2,OWNER,0);
            store32_local(&mut c,2,EPOCH,1);
            store32_const(&mut c,2,STATE,1);
            self.clear_values(&mut c,2);
            self.emit_step(&mut c,2,0);
            lget(&mut c,3);i32c(&mut c,SLOTS as i32);c.push(0x6C);
            i32c(&mut c,(slot+1) as i32);c.push(0x6A);
            c.extend_from_slice(&[0x0F,0x0B]); // return opaque handle; end if
        }
        i32c(&mut c,FULL);c.push(0x0B);c
    }
    // Only a host-authorized completion can make a frame runnable. A stale,
    // duplicate or eager resume while pending is rejected deterministically.
    fn resume(&self)->Vec<u8> {
        let mut c=vec![1,2,I32];Self::checked_frame(&mut c,false);
        let ready_base=self.steps.len()+3;
        for i in 1..=self.steps.len() {
            load32(&mut c,3,STATE);i32c(&mut c,(ready_base+i) as i32);c.push(0x46);
            c.extend_from_slice(&[0x04,0x40]);
            if i==self.steps.len() {
                // RESULT holds the fulfilled external i64, not the future key.
                store32_const(&mut c,3,STATE,(i+1) as i32);
                i32c(&mut c,DONE);
            } else {
                self.emit_step(&mut c,3,i);
                store32_const(&mut c,3,STATE,(i+1) as i32);
                i32c(&mut c,PENDING);
            }
            c.extend_from_slice(&[0x0F,0x0B]);
        }
        i32c(&mut c,INVALID);c.push(0x0B);c
    }
    fn cancel(&self)->Vec<u8> {
        let mut c=vec![1,2,I32];Self::checked_frame(&mut c,false);
        for state in [(self.steps.len()+1) as i32,(self.steps.len()+2) as i32] {
            load32(&mut c,3,STATE);i32c(&mut c,state);c.push(0x46);
            bad_if(&mut c,false);
        }
        store32_const(&mut c,3,STATE,(self.steps.len()+2) as i32);
        self.clear_values(&mut c,3);
        i32c(&mut c,CANCELLED);c.push(0x0B);c
    }
    // The key is a checked i64 identifier compiled from the await operand.
    // No host completion can mutate a frame without a live handle+owner+epoch
    // AND the exact pending key. Numeric host credentials are not cryptographic
    // authority: the JS host must authenticate a single-use external ticket.
    fn complete(&self)->Vec<u8> {
        // Five params (i32,i32,i32,i64,i64); frame ptr and generation
        // locals at indices 5 and 6. The shared check helper is explicitly
        // parametrized with these locals to avoid aliasing the i64 key.
        let mut c=vec![1,2,I32];
        Self::checked_frame_at(&mut c,false,5,6);
        for i in 1..=self.steps.len() {
            load32(&mut c,5,STATE);i32c(&mut c,i as i32);c.push(0x46);
            c.extend_from_slice(&[0x04,0x40]);
            load64(&mut c,5,AWAITED);lget(&mut c,3);
            c.push(0x52); // i64.ne
            bad_if(&mut c,false);
            lget(&mut c,5);lget(&mut c,4);c.push(0x37);
            memarg(&mut c,3,RESULT);
            store32_const(&mut c,5,STATE,(self.steps.len()+3+i) as i32);
            i32c(&mut c,READY);c.push(0x0F);
            c.push(0x0B);
        }
        i32c(&mut c,INVALID);c.push(0x0B);c
    }
    fn pending(&self)->Vec<u8> {
        let mut c=vec![1,2,I32];Self::checked_frame(&mut c,true);
        // API returns the pending key only while a frame is waiting. It never
        // exposes a result or a completed/cancelled frame's key.
        for i in 1..=self.steps.len() {
            load32(&mut c,3,STATE);i32c(&mut c,i as i32);c.push(0x46);
            c.extend_from_slice(&[0x04,0x40]);
            load64(&mut c,3,AWAITED);c.push(0x0F);
            c.push(0x0B);
        }
        c.push(0x00);c.push(0x0B);c
    }
    fn drop_frame(&self)->Vec<u8> {
        let mut c=vec![1,2,I32];Self::checked_frame(&mut c,false);
        self.clear_values(&mut c,3);
        for off in [OWNER,EPOCH,STATE] {
            store32_const(&mut c,3,off,0);
        }
        // GEN is deliberately permanent: retired handles can never alias
        // the next occupant, even when the owner ID or epoch is repeated.
        i32c(&mut c,READY);c.push(0x0B);c
    }
    fn result(&self)->Vec<u8> {
        let mut c=vec![1,2,I32];Self::checked_frame(&mut c,true);
        load32(&mut c,3,STATE);i32c(&mut c,(self.steps.len()+1) as i32);
        c.push(0x47);bad_if(&mut c,true);
        load64(&mut c,3,RESULT);c.push(0x0B);c
    }
    fn capacity(&self)->Vec<u8> {
        let mut c=vec![0];
        i32c(&mut c,SLOTS as i32);c.push(0x0B);c
    }
}
fn i32c(c:&mut Vec<u8>,n:i32){c.push(0x41);sleb(c,n as i64);}
fn i64c(c:&mut Vec<u8>,n:i64){c.push(0x42);sleb(c,n);}
fn lget(c:&mut Vec<u8>,n:u32){c.push(0x20);uleb(c,n as u64);}
fn lset(c:&mut Vec<u8>,n:u32){c.push(0x21);uleb(c,n as u64);}
fn memarg(c:&mut Vec<u8>,align:u8,offset:u32){c.push(align);uleb(c,offset as u64);}
fn load32(c:&mut Vec<u8>,ptr:u32,offset:u32){lget(c,ptr);c.push(0x28);memarg(c,2,offset);}
fn load64(c:&mut Vec<u8>,ptr:u32,offset:u32){lget(c,ptr);c.push(0x29);memarg(c,3,offset);}
fn store32_local(c:&mut Vec<u8>,ptr:u32,offset:u32,local:u32) {
    lget(c,ptr);lget(c,local);c.push(0x36);memarg(c,2,offset);
}
fn store32_const(c:&mut Vec<u8>,ptr:u32,offset:u32,value:i32) {
    lget(c,ptr);i32c(c,value);c.push(0x36);memarg(c,2,offset);
}
fn store64_zero(c:&mut Vec<u8>,ptr:u32,offset:u32) {
    lget(c,ptr);i64c(c,0);c.push(0x37);memarg(c,3,offset);
}
fn bad_if(c:&mut Vec<u8>,trap:bool) {
    c.extend_from_slice(&[0x04,0x40]); // if a typed check failed
    if trap {c.push(0x00);} else {i32c(c,INVALID);c.push(0x0F);}
    c.push(0x0B);
}
