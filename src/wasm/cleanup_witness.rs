//! Emitted *experimental* Wasm lease-cleanup witness, never production mtd.
//!
//! Every emitted cleanup branch has an explicit release before its terminal
//! return/unreachable. This intentionally executes a synthetic checked i64
//! update, NOT the user callback, and holds a Wasm-private simulated lease.
//! A terminated Wasm instance cannot release a separate host-owned lock:
//! host teardown must revoke external leases by actor generation.
use super::*;
use crate::diag::{Diagnostic,Pos};
use crate::target_cleanup_cfg::{CleanupExit,CleanupOp,VerifiedCleanupCfg};
use crate::target_cleanup::StateLeaseMode;

const PANIC_INVALID: i32=7;
const RELEASE: &[u8]=&[
    0x41,0,0x24,1,       // active=0
    0x23,3,0x41,1,0x6a,0x24,3, // releases++
];
const PREFIX: &[u8]=&[
    1,2,I64,            // two i64 locals: old and checked sum
    0x23,1,0x23,4,0x72, // active || closed
    0x04,0x40,          // if
    0x41,7,0x24,0,0x00, // panic_code=7; unreachable BEFORE acquire
    0x0b,
    0x41,1,0x24,1,     // active=1
    0x23,5,0x41,1,0x6a,0x24,5, // acquisitions++
    0x20,1,0x04,0x40,  // if cancel
];
const CANCEL_TAIL: &[u8]=&[0x42,0x7f,0x0f,0x0b]; // i64.const -1; return; end
const CHECKED: &[u8]=&[
    0x23,2,0x21,2,              // old=value
    0x20,2,0x20,0,0x7c,0x21,3, // sum=old+delta (wrapping opcode)
    0x20,2,0x20,3,0x85,         // old ^ sum
    0x20,0,0x20,3,0x85,         // delta ^ sum
    0x83,0x42,0,0x53,           // AND < 0 => signed overflow
    0x04,0x40,                  // if overflow
];
const PANIC_TAIL: &[u8]=&[0x41,1,0x24,0,0x00,0x0b]; // set panic=1; trap; end
const COMMIT: &[u8]=&[0x20,3,0x24,2]; // value = sum
const NORMAL_TAIL: &[u8]=&[0x20,3,0x0b]; // return sum; function end

fn fail(why: &'static str)->Diagnostic {
    Diagnostic::new(Pos::default(),"E_CLEANUP",why)
}
fn certified(cfg:&VerifiedCleanupCfg, exit:CleanupExit)->Result<(),Diagnostic> {
    if cfg.lease_count()!=1 {
        return Err(fail("Wasm cleanup witness requires precisely one state lease"));
    }
    let matching:Vec<_>=cfg.paths().iter().filter(|p|p.lease==0 && p.exit==exit).collect();
    if matching.len()!=1 || !matches!(&matching[0].ops[..],
        [CleanupOp::Acquire { lease:0,mode:StateLeaseMode::ExclusiveWrite },
         CleanupOp::Release { lease:0 }, CleanupOp::Exit(e)] if *e==exit)
    {
        return Err(fail("Wasm cleanup requires verified acquire/release before each exit"));
    }
    Ok(())
}
fn emit_release(cfg:&VerifiedCleanupCfg, exit:CleanupExit, out:&mut Vec<u8>)
    ->Result<(),Diagnostic> {
    certified(cfg,exit)?;
    out.extend_from_slice(RELEASE);
    Ok(())
}
fn emit_run(cfg:&VerifiedCleanupCfg)->Result<Vec<u8>,Diagnostic>{
    let mut out=PREFIX.to_vec();
    emit_release(cfg,CleanupExit::CooperativeCancel,&mut out)?;
    out.extend_from_slice(CANCEL_TAIL);
    out.extend_from_slice(CHECKED);
    emit_release(cfg,CleanupExit::CheckedPanic,&mut out)?;
    out.extend_from_slice(PANIC_TAIL);
    out.extend_from_slice(COMMIT);
    emit_release(cfg,CleanupExit::Normal,&mut out)?;
    out.extend_from_slice(NORMAL_TAIL);
    verify_run_body(&out)?;
    Ok(out)
}

/// Independent emitted-byte attestation. Checks the *whole instruction body*,
/// not caller-writable frontend proof flags or a claimed CFG node. This rejects
/// any omitted, moved, repeated or altered release and any changed branch.
fn verify_run_body(actual:&[u8])->Result<(),Diagnostic>{
    const EXPECTED_LENGTH:usize =
        PREFIX.len()+3*RELEASE.len()+CANCEL_TAIL.len()+CHECKED.len()
        +PANIC_TAIL.len()+COMMIT.len()+NORMAL_TAIL.len();
    if actual.len()!=EXPECTED_LENGTH {
        return Err(fail("emitted Wasm lease cleanup body has incorrect length"));
    }
    let mut at=0usize;
    // The exact known-good executable instruction layout has three distinct
    // release sites: cancel, checked overflow, normal commit.
    for bytes in [PREFIX,RELEASE,CANCEL_TAIL,CHECKED,RELEASE,
        PANIC_TAIL,COMMIT,RELEASE,NORMAL_TAIL] {
        if actual.get(at..at+bytes.len())!=Some(bytes) {
            return Err(fail("emitted Wasm instruction path violates cleanup contract"));
        }
        at+=bytes.len();
    }
    if at!=actual.len() {
        return Err(fail("Wasm cleanup code contains unverified trailing instructions"));
    }
    Ok(())
}
fn read_uleb(bytes:&[u8],at:&mut usize,end:usize)->Result<usize,Diagnostic>{
    let mut out=0usize;
    for shift in (0..35).step_by(7) {
        let b=*bytes.get(*at).ok_or_else(||fail("truncated Wasm section"))?;
        if *at>=end{return Err(fail("out-of-bounds Wasm section"));}
        *at+=1;
        out|=((b&127) as usize)<<shift;
        if b&128==0{return Ok(out);}
    }
    Err(fail("invalid Wasm section length"))
}
/// Validate actual emitted Wasm code-section bytes, not only its planned CFG.
/// This module has exactly six functions; the first is the checked lease run.
pub fn audit_emitted_module(wasm:&[u8])->Result<(),Diagnostic>{
    if wasm.len()<8 || wasm.len()>MAX_OUTPUT || &wasm[..8]!=b"\0asm\x01\0\0\0" {
        return Err(fail("invalid or oversized cleanup Wasm module"));
    }
    let mut offset=8usize;
    let mut found=false;
    while offset<wasm.len() {
        let id=wasm[offset];offset+=1;
        let len=read_uleb(wasm,&mut offset,wasm.len())?;
        let end=offset.checked_add(len).filter(|x|*x<=wasm.len())
            .ok_or_else(||fail("invalid Wasm section extent"))?;
        if id==10 {
            if found{return Err(fail("duplicate Wasm code section"));}
            found=true;
            let mut p=offset;
            if read_uleb(wasm,&mut p,end)?!=6{return Err(fail("wrong number of cleanup functions"));}
            for i in 0..6 {
                let size=read_uleb(wasm,&mut p,end)?;
                let next=p.checked_add(size).filter(|n|*n<=end)
                    .ok_or_else(||fail("invalid Wasm function extent"))?;
                if i==0{verify_run_body(&wasm[p..next])?;}
                p=next;
            }
            if p!=end{return Err(fail("unexpected bytes in Wasm code section"));}
        }
        offset=end;
    }
    if !found{return Err(fail("missing Wasm cleanup code section"));}
    Ok(())
}
fn simple_i32(global:u8)->Vec<u8>{vec![0,0x23,global,0x0b]}
fn simple_i64(global:u8)->Vec<u8>{vec![0,0x23,global,0x0b]}
fn close()->Vec<u8>{
    vec![
        0,0x23,1,0x23,4,0x72,0x45, // !(active||closed)
        0x04,I32,                    // if (result i32)
        0x41,1,0x24,4,0x41,1,       // mark closed, success
        0x05,0x41,0,0x0b,0x0b,      // else 0, end-if, end-function
    ]
}
fn export(out:&mut Vec<u8>,name_str:&str,kind:u8,index:u32) {
    name(out,name_str);out.push(kind);uleb(out,index as u64);
}

/// Only one caller obtains the verified witness artifact: a validated frontend
/// plan lowered through VerifiedCleanupCfg. This is NOT production stateful
/// receiver compilation; no real state field is referenced by this module.
pub(crate) fn emit(cfg:&VerifiedCleanupCfg)->Result<Vec<u8>,Diagnostic>{
    let run=emit_run(cfg)?;
    let mut types=Types(Vec::new());
    let funcs=[
        types.idx(Sig(vec![I64,I32],vec![I64])),
        types.idx(Sig(vec![],vec![I32])),
        types.idx(Sig(vec![],vec![I32])),
        types.idx(Sig(vec![],vec![I32])),
        types.idx(Sig(vec![],vec![I64])),
        types.idx(Sig(vec![],vec![I32])),
    ];
    let mut out=b"\0asm\x01\0\0\0".to_vec();
    let mut ts=Vec::new();
    uleb(&mut ts,types.0.len() as u64);
    for Sig(params,returns) in &types.0 {
        ts.push(0x60);uleb(&mut ts,params.len() as u64);
        ts.extend_from_slice(params);uleb(&mut ts,returns.len() as u64);
        ts.extend_from_slice(returns);
    }
    section(&mut out,1,ts);
    let mut fs=Vec::new();uleb(&mut fs,funcs.len() as u64);
    for f in funcs{uleb(&mut fs,f as u64);}
    section(&mut out,3,fs);
    // 0 panic_code, 1 active lease, 2 synthetic value (initial 41),
    // 3 releases, 4 closed, 5 acquisitions. All are private, mutable globals.
    let mut globals=vec![6];
    for (ty,value) in [(I32,0),(I32,0),(I64,41),(I32,0),(I32,0),(I32,0)] {
        globals.extend_from_slice(&[ty,1,if ty==I64{0x42}else{0x41}]);
        sleb(&mut globals,value);
        globals.push(0x0b);
    }
    section(&mut out,6,globals);
    let mut exports=Vec::new();uleb(&mut exports,7);
    export(&mut exports,"panic_code",3,0);
    for (index,name_str) in ["run_cleanup_witness","lease_active",
        "release_count","acquire_count","state_value","close_witness"].iter().enumerate(){
        export(&mut exports,name_str,0,index as u32);
    }
    section(&mut out,7,exports);
    let bodies=[
        run,simple_i32(1),simple_i32(3),simple_i32(5),
        simple_i64(2),close(),
    ];
    let mut cs=Vec::new();uleb(&mut cs,bodies.len() as u64);
    for b in bodies{uleb(&mut cs,b.len() as u64);cs.extend_from_slice(&b);}
    section(&mut out,10,cs);
    audit_emitted_module(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emitted_byte_auditor_rejects_omission_and_double_release() {
        let mut correct=PREFIX.to_vec();
        correct.extend_from_slice(RELEASE);
        correct.extend_from_slice(CANCEL_TAIL);
        correct.extend_from_slice(CHECKED);
        correct.extend_from_slice(RELEASE);
        correct.extend_from_slice(PANIC_TAIL);
        correct.extend_from_slice(COMMIT);
        correct.extend_from_slice(RELEASE);
        correct.extend_from_slice(NORMAL_TAIL);
        verify_run_body(&correct).unwrap();
        let mut deleted=correct.clone();
        let at=PREFIX.len()+RELEASE.len()+CANCEL_TAIL.len()+CHECKED.len();
        deleted.drain(at..at+RELEASE.len());
        assert_eq!(verify_run_body(&deleted).unwrap_err().code,"E_CLEANUP");
        let mut forged=correct.clone();
        forged[at+2]=0x23; // purported release becomes global.get
        assert_eq!(verify_run_body(&forged).unwrap_err().code,"E_CLEANUP");
        let mut duplicated=correct.clone();
        duplicated.splice(at..at,RELEASE.iter().copied());
        assert_eq!(verify_run_body(&duplicated).unwrap_err().code,"E_CLEANUP");
    }
}
