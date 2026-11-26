//! Test-only Linux x86-64 local authority adapter. Requires protected host
//! directory/ancestors, trusted procfs, local fs flock/fsync semantics and
//! cooperative clients. External current pins remain the trust root; a
//! restored directory plus restored external authority is NOT rollback-proof.
//! No key material is persisted. Rotation must be coordinated with the
//! independently protected current-pin provider before admitting new builds.
use super::*;
use crate::{compiler_cache_key_policy::ActiveKey,compiler_cache_ledger as ledger,sha256::sha256};
const ACTIVE:&str="active";
const PENDING_LEDGER:&str=".ledger.pending";
const LOCK_SH:c_int=1;
const RECORD_MAX:usize=564;
pub(crate) struct Authority{root:LockedRoot}
pub(crate) struct Lease{_lock:File,key:ActiveKey}
impl Lease{pub(crate) fn secret(&self)->[u8;32]{self.key.derived()}}
impl Authority{
    pub(crate) fn open(path:&Path)->io::Result<Self>{
        let root=LockedRoot::open(path).map_err(|_|invalid("invalid authority root"))?;
        let m=root.directory().metadata()?;
        if m.uid()!=unsafe{geteuid()}||m.mode()&0o077!=0{return Err(invalid("authority root must be owner-only"));}
        Ok(Self{root})
    }
    pub(crate) fn distinct_from(&self,store:&ArtifactStore)->io::Result<()>{
        let a=self.root.directory().metadata()?;let b=store.root.directory().metadata()?;
        if a.dev()==b.dev()&&a.ino()==b.ino(){return Err(invalid("authority must not be cache root"));}
        Ok(())
    }
    fn lock(&self,mode:c_int)->io::Result<File>{
        let fd=open_child(self.root.directory(),".",O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC,0)?;
        if unsafe{flock(fd.as_raw_fd(),mode|LOCK_NB)}!=0{return Err(io::Error::last_os_error());}
        Ok(fd)
    }
    fn read(&self)->io::Result<Vec<u8>>{
        let fd=open_child(self.root.directory(),ACTIVE,O_PATH|O_NOFOLLOW|O_CLOEXEC,0)?;
        let meta=fd.metadata()?;
        if !meta.is_file()||meta.nlink()!=1||meta.uid()!=unsafe{geteuid()}||meta.mode()&0o077!=0||meta.len()>RECORD_MAX as u64{
            return Err(invalid("invalid authority record inode"));
        }
        let file=File::open(format!("/proc/self/fd/{}",fd.as_raw_fd()))?;
        let now=file.metadata()?;
        if !now.is_file()||now.dev()!=meta.dev()||now.ino()!=meta.ino(){return Err(invalid("authority record inode changed"));}
        let mut bytes=Vec::new();file.take((RECORD_MAX+1) as u64).read_to_end(&mut bytes)?;
        if bytes.len()>RECORD_MAX{return Err(invalid("authority record too large"));}
        Ok(bytes)
    }
    fn remove_pending(&self)->io::Result<()>{
        let name=CString::new(PENDING_LEDGER).unwrap();
        if unsafe{unlinkat(self.root.directory().as_raw_fd(),name.as_ptr(),0)}==0{return Ok(());}
        let e=io::Error::last_os_error();if e.kind()==io::ErrorKind::NotFound{Ok(())}else{Err(e)}
    }
    // Caller owns exclusive authority lock. No read or signing is admitted
    // during publication. fsync failure after rename is indeterminate.
    fn replace(&self,bytes:&[u8])->io::Result<()>{
        self.remove_pending()?;
        let root=self.root.directory();
        let mut pending=open_child(root,PENDING_LEDGER,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0o600)?;
        let result=(||{
            pending.write_all(bytes)?;pending.sync_all()?;
            let from=CString::new(PENDING_LEDGER).unwrap();let to=CString::new(ACTIVE).unwrap();
            if unsafe{renameat(root.as_raw_fd(),from.as_ptr(),root.as_raw_fd(),to.as_ptr())}!=0{return Err(io::Error::last_os_error());}
            root.sync_all()
        })();
        if result.is_err(){let _=self.remove_pending();}
        result
    }
    pub(crate) fn provision(&self,scope:&str,epoch:u64,master:[u8;32],expected:&[u8;32])->io::Result<()>{
        let _lock=self.lock(LOCK_EX)?;
        match self.read(){
            Err(e) if e.kind()==io::ErrorKind::NotFound=>{},
            Err(e)=>return Err(e),
            Ok(_)=>return Err(invalid("authority already provisioned")),
        }
        let bytes=ledger::record(scope,epoch,master).map_err(invalid)?;
        ledger::admit(scope,&bytes,expected,master).map_err(invalid)?;
        self.replace(&bytes)
    }
    pub(crate) fn lease(&self,scope:&str,expected:&[u8;32],master:[u8;32])->io::Result<Lease>{
        let lock=self.lock(LOCK_SH)?;
        let key=ledger::admit(scope,&self.read()?,expected,master).map_err(invalid)?;
        Ok(Lease{_lock:lock,key})
    }
    pub(crate) fn rotate(&self,scope:&str,old_pin:&[u8;32],old_master:[u8;32],
        next_epoch:u64,next_master:[u8;32],next_pin:&[u8;32])->io::Result<()>{
        let _lock=self.lock(LOCK_EX)?;
        let mut active=ledger::admit(scope,&self.read()?,old_pin,old_master).map_err(invalid)?;
        active.rotate(next_epoch,next_master).map_err(invalid)?;
        let bytes=ledger::record(scope,next_epoch,next_master).map_err(invalid)?;
        if &sha256(&bytes)!=next_pin{return Err(invalid("next authority pin mismatch"));}
        self.replace(&bytes)
    }
}
