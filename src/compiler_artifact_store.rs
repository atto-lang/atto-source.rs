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
        // Only a structurally valid, authenticated occupant of this slot
        // may count as a collision. A corrupted key field is a diagnostic.
        let actual: [u8;32]=bytes[8..40].try_into().map_err(|_|invalid("cache key length"))?;
        if slot(&actual)!=slot(key){return Err(invalid("cache slot mismatch"));}
        let payload=receipt::verify(&actual,&bytes,secret).map_err(invalid)?;
        if &actual!=key{return Ok(None);}
        Ok(Some(payload))
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
            // A directory fsync failure after rename is an indeterminate
            // durability result, not a claim that the old slot survived.
            // Any reader still authenticates the complete returned bytes.
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
        let mut altered_key=data.clone();altered_key[39]^=1;
        fs::write(&path,&altered_key).unwrap();assert!(store.load(&KEY,&SECRET).is_err());
        let mut wrong_slot=data.clone();wrong_slot[8]^=0x80;
        fs::write(&path,&wrong_slot).unwrap();assert!(store.load(&KEY,&SECRET).is_err());
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

    // Fixture-only compiler integration. No public cache-load CLI or production
    // key provisioning. Bind to the actual test executable and exact literal
    // source; no ambient imports or user-configurable target flags in this lane.
    const GUEST: &str = "pub fnc main(): int { return 6 * 7; }";
    fn guest_key(source:&str)->[u8;32] {
        use crate::{compiler_cache_key as k,sha256::hex_digest};
        let mut binary=Vec::new();
        File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut binary).unwrap();
        assert!(binary.len()<=128*1024*1024);
        let compiler=hex_digest(&binary);
        let toolchain=hex_digest(b"fixture: same executable contains compiler and code emitter");
        let target=hex_digest(format!("fixture: wasm32/{}; posix",crate::ABI_VERSION).as_bytes());
        let config=hex_digest(b"fixture: literal scalar source; no host imports; compile()");
        let sources=hex_digest(source.as_bytes());
        let bindings=hex_digest(b"fixture: no imports");
        let key=k::encode(&k::Input{stage:"wasm-module",owner:"@test/cache/fixture",
            unit_kind:"source-unit",unit:"main.atto",compiler:&compiler,toolchain:&toolchain,
            target:&target,configuration:&config,sources:&sources,bindings:&bindings,
            generated:None,dependencies:&[]}).unwrap();
        let mut result=[0u8;32];
        for (i,byte) in result.iter_mut().enumerate(){
            *byte=u8::from_str_radix(&key[i*2..i*2+2],16).unwrap();
        }
        result
    }
    fn execute_wasm(path:&Path){
        let script="const fs=require('node:fs'),assert=require('node:assert/strict');const b=fs.readFileSync(process.argv[1]);assert.ok(WebAssembly.validate(b));const m=new WebAssembly.Module(b);assert.deepEqual(WebAssembly.Module.imports(m),[]);const i=new WebAssembly.Instance(m,{});assert.equal(i.exports.main(),42n);assert.equal(i.exports.panic_code.value,0);console.log('CACHE_WASM_EXECUTED|result=42')";
        let status=Command::new("node").args(["-e",script]).arg(path).status().unwrap();
        assert!(status.success());
    }
    #[test]
    fn compiled_guest_cross_process_fixture(){
        let t=Temp::new();
        let store=ArtifactStore::open(&t.0).unwrap();
        let key=guest_key(GUEST);
        let changed=guest_key("pub fnc main(): int { return 43; }");
        assert_ne!(key,changed);
        assert!(store.load(&key,&SECRET).unwrap().is_none());
        let cold=crate::compile(GUEST).expect("real compiler rejected fixture");
        store.publish(&key,&receipt::seal(&key,&cold,&SECRET).unwrap(),&SECRET).unwrap();
        let cold_path=t.0.join("cold.wasm");
        fs::write(&cold_path,&cold).unwrap();execute_wasm(&cold_path);
        assert!(store.load(&changed,&SECRET).unwrap().is_none());
        let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","compiler_artifact_store::tests::compiled_guest_child","--nocapture"])
            .env("ATTO_CACHE_GUEST_ROOT",&t.0).status().unwrap();
        assert!(status.success());
        let warm=fs::read(t.0.join("warm.wasm")).unwrap();
        assert_eq!(cold,warm);
        assert_eq!(cold,crate::compile(GUEST).unwrap());
        println!("CACHE_GUEST_FIXTURE|cold=compiled|warm=loaded-in-second-process|byte-identical|result=42");
    }
    #[test]
    fn compiled_guest_child(){
        if let Some(root)=std::env::var_os("ATTO_CACHE_GUEST_ROOT"){
            let root=Path::new(&root);
            let store=ArtifactStore::open(root).unwrap();
            let bytes=store.load(&guest_key(GUEST),&SECRET).unwrap().expect("expected authenticated hit");
            let path=root.join("warm.wasm");
            fs::write(&path,bytes).unwrap();
            execute_wasm(&path);
            // This child deliberately never invokes the compiler.
            println!("CACHE_GUEST_CHILD|verified-hit|no-compile");
        }
    }

    const MANIFEST: &str = "[package]\nname=\"shop\"\nversion=\"1.0.0\"\n";
    const MAIN: &str = r#"use namespace shop;
import module Orders from "+lib/orders.atto";
pub fnc main(): int { return Orders.total(21); }"#;
    const ORDERS: &str = r#"use namespace sales;
import module Math from "./math.atto";
export pub module Orders;
define module Orders as
 pub fnc total(int x): int { return Math.double(x); }
end"#;
    const MATH: &str = r#"use namespace math;
export pub[pkg] module Math;
define module Math as
 pub fnc double(int x): int { return x * 2; }
end"#;
    fn package_inputs<'a>(math:&'a str)->[crate::package_resolver::SnapshotSource<'a>;3]{
        use crate::package_resolver::SnapshotSource;
        [SnapshotSource{path:"src/main.atto",source:MAIN},
         SnapshotSource{path:"lib/orders.atto",source:ORDERS},
         SnapshotSource{path:"lib/math.atto",source:math}]
    }
    fn package_lock(inputs:&[crate::package_resolver::SnapshotSource<'_>])->String{
        use crate::sha256::hex_digest;
        let mut lock=format!("[lock]\nmanifest_sha256=\"{}\"\n[files]\n",hex_digest(MANIFEST.as_bytes()));
        for i in inputs{lock.push_str(&format!("\"{}\"=\"{}\"\n",i.path,hex_digest(i.source.as_bytes())));}
        lock
    }
    fn prepare_package(inputs:&[crate::package_resolver::SnapshotSource<'_>],lock:&str,pin:&str)
        -> Result<([u8;32],String),crate::Diagnostic>{
        use crate::{package_resolver::prepare_locked_snapshot,compiler_cache_key as k,sha256::hex_digest};
        let prepared=prepare_locked_snapshot("src/main.atto",MANIFEST,lock,pin,inputs)?;
        // Only the controlled fixture supplies these stage witnesses. Hash
        // the whole host test binary (compiler, embedded libraries, emitter)
        // rather than accepting a caller's claimed compiler version.
        let mut binary=Vec::new();
        File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut binary).unwrap();
        assert!(binary.len()<=128*1024*1024);
        let compiler=hex_digest(&binary);
        let toolchain=hex_digest(b"fixture: same executable includes complete compiler");
        let target=hex_digest(format!("fixture: wasm32/{}; {:?}",crate::ABI_VERSION,crate::library::target_os()).as_bytes());
        let config=hex_digest(b"fixture: locked snapshot; no external host grants");
        let bindings=hex_digest(prepared.linked.as_bytes());
        let key=k::encode(&k::Input{stage:"wasm-module",owner:"shop@1.0.0",
            unit_kind:"source-unit",unit:"src/main.atto",compiler:&compiler,toolchain:&toolchain,
            target:&target,configuration:&config,sources:pin,bindings:&bindings,generated:None,
            dependencies:&[]}).unwrap();
        let mut result=[0u8;32];
        for (i,byte) in result.iter_mut().enumerate(){*byte=u8::from_str_radix(&key[i*2..i*2+2],16).unwrap();}
        Ok((result,prepared.linked))
    }
    #[test]
    fn locked_package_cache_fixture(){
        use crate::sha256::hex_digest;
        let t=Temp::new();let store=ArtifactStore::open(&t.0).unwrap();
        let inputs=package_inputs(MATH);let lock=package_lock(&inputs);let pin=hex_digest(lock.as_bytes());
        let (key,linked)=prepare_package(&inputs,&lock,&pin).unwrap();
        assert!(store.load(&key,&SECRET).unwrap().is_none());
        // Publish only after real compiler semantic checks and emission.
        let cold=crate::compile(&linked).unwrap();
        assert_eq!(cold,crate::package_resolver::compile_locked_snapshot("src/main.atto",MANIFEST,&lock,&pin,&inputs).unwrap());
        store.publish(&key,&receipt::seal(&key,&cold,&SECRET).unwrap(),&SECRET).unwrap();
        fs::write(t.0.join("package-cold.wasm"),&cold).unwrap();
        execute_wasm(&t.0.join("package-cold.wasm"));
        let reordered=[inputs[2],inputs[0],inputs[1]];
        assert_eq!(prepare_package(&reordered,&lock,&pin).unwrap().0,key);
        let edited=MATH.replace("x * 2","x * 3");
        let changed=package_inputs(&edited);
        assert_eq!(prepare_package(&changed,&lock,&pin).unwrap_err().code,"E_LOCK_HASH");
        assert_eq!(prepare_package(&inputs[..2],&lock,&pin).unwrap_err().code,"E_LOCK_HASH");
        assert_eq!(prepare_package(&[inputs[0],inputs[0],inputs[2]],&lock,&pin).unwrap_err().code,"E_DUPLICATE");
        assert_eq!(prepare_package(&inputs,&lock,&"0".repeat(64)).unwrap_err().code,"E_LOCK_HASH");
        let changed_lock=package_lock(&changed);let changed_pin=hex_digest(changed_lock.as_bytes());
        let (new_key,new_linked)=prepare_package(&changed,&changed_lock,&changed_pin).unwrap();
        assert_ne!(key,new_key);assert!(store.load(&new_key,&SECRET).unwrap().is_none());
        assert_ne!(cold,crate::compile(&new_linked).unwrap());
        let private=MATH.replace("export pub[pkg] module Math;","");
        let denied=package_inputs(&private);let denied_lock=package_lock(&denied);
        assert!(prepare_package(&denied,&denied_lock,&hex_digest(denied_lock.as_bytes())).is_err());
        assert_eq!(store.load(&key,&SECRET).unwrap().unwrap(),cold);
        let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","compiler_artifact_store::tests::locked_package_child","--nocapture"])
            .env("ATTO_CACHE_PACKAGE_ROOT",&t.0).status().unwrap();assert!(status.success());
        assert_eq!(fs::read(t.0.join("package-warm.wasm")).unwrap(),cold);
        println!("CACHE_LOCKED_PACKAGE|three-files|revalidated-graph|cold-warm-byte-identical|stale-pins-and-private-denied");
    }
    #[test]
    fn locked_package_child(){
        if let Some(root)=std::env::var_os("ATTO_CACHE_PACKAGE_ROOT"){
            use crate::sha256::hex_digest;
            let inputs=package_inputs(MATH);let lock=package_lock(&inputs);let pin=hex_digest(lock.as_bytes());
            let root=Path::new(&root);
            let (key,_)=prepare_package(&inputs,&lock,&pin).unwrap();
            let store=ArtifactStore::open(root).unwrap();
            let bytes=store.load(&key,&SECRET).unwrap().expect("authenticated hit after graph revalidation");
            let path=root.join("package-warm.wasm");
            fs::write(&path,bytes).unwrap();execute_wasm(&path);
            println!("CACHE_LOCKED_CHILD|graph-admitted|verified-hit|no-compile");
        }
    }
}
