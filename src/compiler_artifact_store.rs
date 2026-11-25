//! Linux-only candidate bounded disk store. Fixed 16 direct-mapped slots
//! (64 MiB payload maximum + headers and one pending entry). The configured
//! directory and ancestors must be protected by the host; same-UID malicious
//! processes and arbitrary files written externally are outside the quota
//! guarantee. No guest execution, source verification or key management.
use crate::{compiler_artifact_receipt as receipt, secure_fs::LockedRoot};
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::raw::{c_char, c_int};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 1;
const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;
const O_DIRECTORY: c_int = 0o200000;
const O_NOFOLLOW: c_int = 0o400000;
const O_CLOEXEC: c_int = 0o2000000;
const O_PATH: c_int = 0o10000000;
const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;
const PENDING: &str = ".pending";
extern "C" {
    fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, ...) -> c_int;
    fn unlinkat(dirfd: c_int, pathname: *const c_char, flags: c_int) -> c_int;
    fn renameat(oldfd: c_int, old: *const c_char, newfd: c_int, new: *const c_char) -> c_int;
    fn flock(fd: c_int, operation: c_int) -> c_int;
    fn geteuid() -> u32;
}
fn invalid(detail: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData,detail) }
fn slot(key: &[u8;32]) -> String { format!("slot-{:x}",key[0] >> 4) }
fn open_child(root:&File,name:&str,flags:c_int,mode:c_int)->io::Result<File>{
    let name=CString::new(name).map_err(|_|invalid("invalid slot name"))?;
    // SAFETY: NUL-terminated fixed name, valid directory descriptor, mode
    // supplied for O_CREAT; successful descriptor transferred to File.
    let fd=unsafe{openat(root.as_raw_fd(),name.as_ptr(),flags,mode)};
    if fd<0 {return Err(io::Error::last_os_error());}
    Ok(unsafe{File::from_raw_fd(fd)})
}
fn remove_pending(root:&File)->io::Result<()> {
    let name=CString::new(PENDING).unwrap();
    // SAFETY: pinned directory and fixed name; unlink never follows symlinks.
    if unsafe{unlinkat(root.as_raw_fd(),name.as_ptr(),0)}==0{return Ok(());}
    let e=io::Error::last_os_error();if e.kind()==io::ErrorKind::NotFound{Ok(())}else{Err(e)}
}
pub struct ArtifactStore { root:LockedRoot }
impl ArtifactStore {
    pub fn open(path:&Path)->io::Result<Self>{
        let root=LockedRoot::open(path).map_err(|_|invalid("invalid cache root"))?;
        let meta=root.directory().metadata()?;
        // SAFETY: geteuid has no arguments or memory effects.
        if meta.uid()!=unsafe{geteuid()}||meta.mode()&0o077!=0{return Err(invalid("cache directory must be owner-only"));}
        Ok(Self{root})
    }
    pub fn load(&self,key:&[u8;32],secret:&[u8;32])->io::Result<Option<Vec<u8>>>{
        let root=self.root.directory();
        // O_PATH never opens a device or blocks on FIFO; stat the actual
        // descriptor before opening a regular inode through trusted procfs.
        let pinned=match open_child(root,&slot(key),O_PATH|O_NOFOLLOW|O_CLOEXEC,0){
            Ok(file)=>file,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e)
        };
        let meta=pinned.metadata()?;
        if !meta.is_file()||meta.nlink()!=1||meta.len()>(receipt::HEADER+receipt::MAX_ARTIFACT) as u64{
            return Err(invalid("not a bounded regular cache file"));
        }
        // Host must provide a trusted Linux /proc mount. Reopen the pinned
        // inode, not its replaceable name, and verify identity on the new FD.
        let file=File::open(format!("/proc/self/fd/{}",pinned.as_raw_fd()))?;
        let opened=file.metadata()?;
        if !opened.is_file()||opened.dev()!=meta.dev()||opened.ino()!=meta.ino(){
            return Err(invalid("cache descriptor identity changed"));
        }
        let mut bytes=Vec::new();
        file.take((receipt::HEADER+receipt::MAX_ARTIFACT+1) as u64).read_to_end(&mut bytes)?;
        if bytes.len()<receipt::HEADER||bytes.len()>receipt::HEADER+receipt::MAX_ARTIFACT{
            return Err(invalid("cache file size"));
        }
        // A collision is a miss, not authorization to reuse another key.
        if &bytes[8..40]!=key{return Ok(None);}
        receipt::verify(key,&bytes,secret).map(Some).map_err(invalid)
    }
    pub fn publish(&self,key:&[u8;32],envelope:&[u8],secret:&[u8;32])->io::Result<()>{
        // Verify before touching an existing entry; no untrusted partial write.
        receipt::verify(key,envelope,secret).map_err(invalid)?;
        let root=self.root.directory();
        // Independent open-file description for each writer, even threads
        // sharing this store. Advisory lock serializes cooperating publishers.
        let lock=open_child(root,".",O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC,0)?;
        if unsafe{flock(lock.as_raw_fd(),LOCK_EX|LOCK_NB)}!=0{return Err(io::Error::last_os_error());}
        // Under the lock, stale pending from an interrupted writer is removed.
        remove_pending(root)?;
        let mut file=open_child(root,PENDING,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0o600)?;
        let result=(|| {
            file.write_all(envelope)?;file.sync_all()?;
            let from=CString::new(PENDING).unwrap();let to=CString::new(slot(key)).unwrap();
            // SAFETY: both fixed names are relative to the same pinned root.
            if unsafe{renameat(root.as_raw_fd(),from.as_ptr(),root.as_raw_fd(),to.as_ptr())}!=0{
                return Err(io::Error::last_os_error());
            }
            root.sync_all()
        })();
        if result.is_err(){let _=remove_pending(root);}
        result
        // Dropping lock closes the independent FD and releases flock.
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_artifact_receipt::tests::{KEY,SECRET,PAYLOAD};
    use std::{fs,os::unix::fs::{DirBuilderExt,symlink},process::Command,sync::atomic::{AtomicUsize,Ordering}};
    static NEXT:AtomicUsize=AtomicUsize::new(0);
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new()->Self{
            let path=std::env::temp_dir().join(format!("atto-cache-test-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();Self(path)
        }
    }
    impl Drop for Temp{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    #[test]
    fn disk_roundtrip_restart_tampering_and_eviction(){
        let t=Temp::new();let store=ArtifactStore::open(&t.0).unwrap();
        let data=receipt::seal(&KEY,PAYLOAD,&SECRET).unwrap();
        assert!(store.load(&KEY,&SECRET).unwrap().is_none());
        store.publish(&KEY,&data,&SECRET).unwrap();
        let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","compiler_artifact_store::tests::child_read","--nocapture"])
            .env("ATTO_CACHE_TEST_CHILD_ROOT",&t.0).status().unwrap();
        assert!(status.success());
        let path=t.0.join(slot(&KEY));
        let mut forged=data.clone();forged[112]^=1;
        let hash=crate::sha256::sha256(&forged[112..]);forged[48..80].copy_from_slice(&hash);
        assert!(store.publish(&KEY,&forged,&SECRET).is_err());
        assert_eq!(store.load(&KEY,&SECRET).unwrap().unwrap(),PAYLOAD);
        fs::write(&path,&forged).unwrap();assert!(store.load(&KEY,&SECRET).is_err());
        fs::write(t.0.join(PENDING),b"partial").unwrap();
        store.publish(&KEY,&data,&SECRET).unwrap();
        assert!(!t.0.join(PENDING).exists());
        assert_eq!(store.load(&KEY,&SECRET).unwrap().unwrap(),PAYLOAD);
        let mut other=KEY;other[31]^=1;
        store.publish(&other,&receipt::seal(&other,b"other",&SECRET).unwrap(),&SECRET).unwrap();
        assert!(store.load(&KEY,&SECRET).unwrap().is_none());
        assert_eq!(store.load(&other,&SECRET).unwrap().unwrap(),b"other");
        println!("CACHE_STORE_RUST|second-process|forgery|interrupted-writer|eviction");
    }
    #[test]
    fn child_read(){
        if let Some(root)=std::env::var_os("ATTO_CACHE_TEST_CHILD_ROOT"){
            let store=ArtifactStore::open(Path::new(&root)).unwrap();
            assert_eq!(store.load(&KEY,&SECRET).unwrap().unwrap(),PAYLOAD);
            println!("CACHE_STORE_CHILD|verified-payload");
        }
    }
    #[test]
    fn rejects_symlink_fifo_directory_hardlink_and_oversized(){
        let t=Temp::new();let store=ArtifactStore::open(&t.0).unwrap();
        let path=t.0.join(slot(&KEY));let outside=t.0.join("fixture");
        fs::write(&outside,receipt::seal(&KEY,PAYLOAD,&SECRET).unwrap()).unwrap();
        symlink(&outside,&path).unwrap();assert!(store.load(&KEY,&SECRET).is_err());fs::remove_file(&path).unwrap();
        fs::hard_link(&outside,&path).unwrap();assert!(store.load(&KEY,&SECRET).is_err());fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();assert!(store.load(&KEY,&SECRET).is_err());fs::remove_dir(&path).unwrap();
        assert!(Command::new("mkfifo").arg(&path).status().unwrap().success());
        assert!(store.load(&KEY,&SECRET).is_err());fs::remove_file(&path).unwrap();
        File::create(&path).unwrap().set_len((receipt::HEADER+receipt::MAX_ARTIFACT+1) as u64).unwrap();
        assert!(store.load(&KEY,&SECRET).is_err());
        println!("CACHE_STORE_RUST|symlink|hardlink|directory|fifo|oversized-rejected");
    }
    #[test]
    fn root_swap_and_writer_lock(){
        let t=Temp::new();let real=t.0.join("real");
        fs::DirBuilder::new().mode(0o700).create(&real).unwrap();
        let store=ArtifactStore::open(&real).unwrap();let data=receipt::seal(&KEY,PAYLOAD,&SECRET).unwrap();
        store.publish(&KEY,&data,&SECRET).unwrap();
        let retired=t.0.join("retired");fs::rename(&real,&retired).unwrap();
        symlink(&retired,&real).unwrap();assert!(ArtifactStore::open(&real).is_err());
        assert_eq!(store.load(&KEY,&SECRET).unwrap().unwrap(),PAYLOAD);
        let lock=open_child(store.root.directory(),".",O_RDONLY|O_DIRECTORY|O_CLOEXEC,0).unwrap();
        assert_eq!(unsafe{flock(lock.as_raw_fd(),LOCK_EX|LOCK_NB)},0);
        assert_eq!(store.publish(&KEY,&data,&SECRET).unwrap_err().kind(),io::ErrorKind::WouldBlock);
        drop(lock);store.publish(&KEY,&data,&SECRET).unwrap();
    }
}
