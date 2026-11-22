//! Strict single-instance AOT continuation profile. No JSPI inside v3 exports.
//! Only scalar literal awaits ending in an explicit literal return-await are
//! admitted. One mutable-global frame per Wasm instance, bounded to 8 awaits.
//! The trusted host supplies owner/epoch identity; NOT cryptographic authority.
//! Status READY=0, PENDING=1, DONE=2, CANCELLED=4, INVALID=-1.
use super::{name, sleb, uleb, I32, I64, Sig, TExpr, TProgram, TStmt, Types};
use crate::ast::Type;
use crate::check::Import;

const OWNER:u32=1; const EPOCH:u32=2; const STATE:u32=3;
const LAST_EPOCH:u32=4; const RESULT:u32=5;
const INVALID:i64=-1; const PENDING:i64=1; const DONE:i64=2; const CANCELLED:i64=4;

pub(super) struct PortableTaskV3 { awaited:Vec<i64> }
impl PortableTaskV3 {
    pub(super) fn from_program(p:&TProgram)->Option<Self> {
        if p.funcs.len()!=1 || p.main!=0 || !p.tasks.is_empty() ||
           !p.generators.is_empty() || !p.actors.is_empty() ||
           !p.globals.is_empty() || p.heap || p.strings || p.guest_eh ||
           !p.data.is_empty() || p.imports!=[Import::Cooperate] {return None;}
        let main=&p.funcs[0];
        if main.name!="main" || !main.params.is_empty() || !main.locals.is_empty() ||
           main.ret!=Type::Int || main.body.is_empty() || main.body.len()>8 {return None;}
        let mut awaited=Vec::new();
        for (i,stmt) in main.body.iter().enumerate() {
            let e=match stmt {
                TStmt::Expr(v) if i+1<main.body.len()=>v,
                TStmt::Return(Some(v)) if i+1==main.body.len()=>v,
                _=>return None,
            };
            let TExpr::AwaitYield(inner)=&e.e else {return None};
            if e.ty!=Type::Int || inner.ty!=Type::Int {return None;}
            let TExpr::I64(v)=inner.e else {return None};
            awaited.push(v);
        }
        Some(Self{awaited})
    }
    pub(super) fn global_defs(&self,globals:&mut Vec<u8>) {
        // owner, epoch, resume PC, permanent epoch tombstone, result slot
        for _ in 0..4 {globals.extend_from_slice(&[I32,1,0x41,0,0x0B]);}
        globals.extend_from_slice(&[I64,1,0x42,0,0x0B]);
    }
    pub(super) fn append_functions(&self,types:&mut Types,
        function_types:&mut Vec<u32>,bodies:&mut Vec<Vec<u8>>) {
        let signatures=[
            Sig(vec![I32,I32],vec![I32]), Sig(vec![I32,I32],vec![I32]),
            Sig(vec![I32,I32],vec![I32]), Sig(vec![I32,I32],vec![I32]),
            Sig(vec![I32,I32],vec![I64]),
        ];
        let codes=[self.start(),self.resume(),self.cancel(),self.drop_frame(),self.result()];
        for (sig,code) in signatures.into_iter().zip(codes) {
            function_types.push(types.idx(sig));
            bodies.push(code);
        }
    }
    pub(super) fn append_exports(&self,exports:&mut Vec<u8>,first:u32) {
        for (i,exported) in ["task_start_v3","task_resume_v3","task_cancel_v3",
                             "task_drop_v3","task_result_v3"].into_iter().enumerate() {
            name(exports,exported);exports.push(0);uleb(exports,first as u64+i as u64);
        }
    }
    pub(super) fn append_names(&self,names:&mut Vec<String>) {
        names.extend(["atto.task_start_v3","atto.task_resume_v3","atto.task_cancel_v3",
                      "atto.task_drop_v3","atto.task_result_v3"].into_iter().map(str::to_owned));
    }
    fn start(&self)->Vec<u8> {
        let mut c=vec![0]; // no locals
        gget(&mut c,STATE);i32c(&mut c,0);c.push(0x47);invalid_if(&mut c);
        for param in [0,1] {
            lget(&mut c,param);i32c(&mut c,0);c.push(0x4C);invalid_if(&mut c);
        }
        lget(&mut c,1);gget(&mut c,LAST_EPOCH);c.push(0x4C);invalid_if(&mut c);
        lget(&mut c,0);gset(&mut c,OWNER);
        lget(&mut c,1);gset(&mut c,EPOCH);
        lget(&mut c,1);gset(&mut c,LAST_EPOCH);
        i32c(&mut c,1);gset(&mut c,STATE);
        i64c(&mut c,0);gset(&mut c,RESULT);
        i32c(&mut c,PENDING);c.push(0x0B);c
    }
    fn check_owner(&self,c:&mut Vec<u8>) {
        gget(c,STATE);c.push(0x45);invalid_if(c); // no frame
        lget(c,0);gget(c,OWNER);c.push(0x47);invalid_if(c);
        lget(c,1);gget(c,EPOCH);c.push(0x47);invalid_if(c);
    }
    fn resume(&self)->Vec<u8> {
        let mut c=vec![0];self.check_owner(&mut c);
        for i in 1..=self.awaited.len() {
            gget(&mut c,STATE);i32c(&mut c,i as i64);c.push(0x46);
            c.extend_from_slice(&[0x04,0x40]);
            if i==self.awaited.len() {
                i64c(&mut c,self.awaited[i-1]);gset(&mut c,RESULT);
                i32c(&mut c,(i+1) as i64);gset(&mut c,STATE);
                i32c(&mut c,DONE);
            } else {
                i32c(&mut c,(i+1) as i64);gset(&mut c,STATE);
                i32c(&mut c,PENDING);
            }
            c.extend_from_slice(&[0x0F,0x0B]); // return status, close if
        }
        i32c(&mut c,INVALID);c.push(0x0B);c
    }
    fn cancel(&self)->Vec<u8> {
        let mut c=vec![0];self.check_owner(&mut c);
        gget(&mut c,STATE);i32c(&mut c,self.awaited.len() as i64);
        c.push(0x4B);invalid_if(&mut c); // completed or cancelled
        i32c(&mut c,(self.awaited.len()+2) as i64);gset(&mut c,STATE);
        i64c(&mut c,0);gset(&mut c,RESULT);
        i32c(&mut c,CANCELLED);c.push(0x0B);c
    }
    fn drop_frame(&self)->Vec<u8> {
        let mut c=vec![0];self.check_owner(&mut c);
        i64c(&mut c,0);gset(&mut c,RESULT);
        for g in [OWNER,EPOCH,STATE] {i32c(&mut c,0);gset(&mut c,g);}
        // LAST_EPOCH never resets: reused generations and stale tickets denied.
        i32c(&mut c,0);c.push(0x0B);c
    }
    fn result(&self)->Vec<u8> {
        let mut c=vec![0];
        gget(&mut c,STATE);i32c(&mut c,(self.awaited.len()+1) as i64);
        c.push(0x47);trap_if(&mut c);
        lget(&mut c,0);gget(&mut c,OWNER);c.push(0x47);trap_if(&mut c);
        lget(&mut c,1);gget(&mut c,EPOCH);c.push(0x47);trap_if(&mut c);
        gget(&mut c,RESULT);c.push(0x0B);c
    }
}
fn i32c(c:&mut Vec<u8>,v:i64){c.push(0x41);sleb(c,v);}
fn i64c(c:&mut Vec<u8>,v:i64){c.push(0x42);sleb(c,v);}
fn lget(c:&mut Vec<u8>,n:u32){c.push(0x20);uleb(c,n as u64);}
fn gget(c:&mut Vec<u8>,n:u32){c.push(0x23);uleb(c,n as u64);}
fn gset(c:&mut Vec<u8>,n:u32){c.push(0x24);uleb(c,n as u64);}
fn invalid_if(c:&mut Vec<u8>){c.extend_from_slice(&[0x04,0x40]);i32c(c,INVALID);c.extend_from_slice(&[0x0F,0x0B]);}
fn trap_if(c:&mut Vec<u8>){c.extend_from_slice(&[0x04,0x40,0x00,0x0B]);}
