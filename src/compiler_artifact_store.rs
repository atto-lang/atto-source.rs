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
#[cfg(any(test, feature = "experimental-incremental-cache"))]
pub(crate) mod authority;
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
    fn execute_wasm(path:&Path){execute_wasm_result(path,"42")}
    fn execute_wasm_result(path:&Path,expected:&str){
        let script="const fs=require('node:fs'),assert=require('node:assert/strict');const b=fs.readFileSync(process.argv[1]);assert.ok(WebAssembly.validate(b));const m=new WebAssembly.Module(b);assert.deepEqual(WebAssembly.Module.imports(m),[]);const i=new WebAssembly.Instance(m,{});assert.equal(i.exports.main(),BigInt(process.argv[2]));assert.equal(i.exports.panic_code.value,0);console.log('CACHE_WASM_EXECUTED|result='+process.argv[2])";
        let status=Command::new("node").args(["-e",script]).arg(path).arg(expected).status().unwrap();
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
    fn prepare_package_at(inputs:&[crate::package_resolver::SnapshotSource<'_>],lock:&str,pin:&str,
        target:crate::library::TargetOs)->Result<crate::compiler_cache_witness::Prepared,crate::Diagnostic>{
        use crate::sha256::hex_digest;
        // The test process is the complete statically linked compiler fixture.
        // Production hosts must separately attest their actual build closure.
        let mut binary=Vec::new();
        File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut binary).unwrap();
        assert!(binary.len()<=128*1024*1024);
        let compiler=hex_digest(&binary);
        let toolchain=hex_digest(b"fixture: same executable includes complete compiler");
        crate::compiler_cache_witness::prepare("src/main.atto",MANIFEST,lock,pin,inputs,target,&compiler,&toolchain)
    }
    fn prepare_package(inputs:&[crate::package_resolver::SnapshotSource<'_>],lock:&str,pin:&str)
        -> Result<crate::compiler_cache_witness::Prepared,crate::Diagnostic>{
        prepare_package_at(inputs,lock,pin,crate::library::TargetOs::Posix)
    }
    #[test]
    fn locked_package_cache_fixture(){
        use crate::sha256::hex_digest;
        let t=Temp::new();let store=ArtifactStore::open(&t.0).unwrap();
        let inputs=package_inputs(MATH);let lock=package_lock(&inputs);let pin=hex_digest(lock.as_bytes());
        let prepared=prepare_package(&inputs,&lock,&pin).unwrap();
        let key=prepared.key;
        let windows=prepare_package_at(&inputs,&lock,&pin,crate::library::TargetOs::Windows).unwrap();
        assert_ne!(key,windows.key);
        assert!(store.load(&windows.key,&SECRET).unwrap().is_none());
        assert!(store.load(&key,&SECRET).unwrap().is_none());
        // Publish only after real compiler semantic checks and emission.
        let cold=prepared.compile().unwrap();
        assert_eq!(cold,crate::package_resolver::compile_locked_snapshot("src/main.atto",MANIFEST,&lock,&pin,&inputs).unwrap());
        store.publish(&key,&receipt::seal(&key,&cold,&SECRET).unwrap(),&SECRET).unwrap();
        fs::write(t.0.join("package-cold.wasm"),&cold).unwrap();
        execute_wasm(&t.0.join("package-cold.wasm"));
        let reordered=[inputs[2],inputs[0],inputs[1]];
        assert_eq!(prepare_package(&reordered,&lock,&pin).unwrap().key,key);
        let edited=MATH.replace("x * 2","x * 3");
        let changed=package_inputs(&edited);
        assert_eq!(prepare_package(&changed,&lock,&pin) .err().expect("expected diagnostic").code,"E_LOCK_HASH");
        assert_eq!(prepare_package(&inputs[..2],&lock,&pin) .err().expect("expected diagnostic").code,"E_LOCK_HASH");
        assert_eq!(prepare_package(&[inputs[0],inputs[0],inputs[2]],&lock,&pin) .err().expect("expected diagnostic").code,"E_DUPLICATE");
        assert_eq!(prepare_package(&inputs,&lock,&"0".repeat(64)) .err().expect("expected diagnostic").code,"E_LOCK_HASH");
        let changed_lock=package_lock(&changed);let changed_pin=hex_digest(changed_lock.as_bytes());
        let updated=prepare_package(&changed,&changed_lock,&changed_pin).unwrap();
        assert_ne!(key,updated.key);assert!(store.load(&updated.key,&SECRET).unwrap().is_none());
        assert_ne!(cold,updated.compile().unwrap());
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
            let key=prepare_package(&inputs,&lock,&pin).unwrap().key;
            let store=ArtifactStore::open(root).unwrap();
            let bytes=store.load(&key,&SECRET).unwrap().expect("authenticated hit after graph revalidation");
            let path=root.join("package-warm.wasm");
            fs::write(&path,bytes).unwrap();execute_wasm(&path);
            println!("CACHE_LOCKED_CHILD|graph-admitted|verified-hit|no-compile");
        }
    }

    #[test]
    fn locked_package_stage_timing_smoke(){
        use crate::{compiler_cache_witness as witness,library::TargetOs,sha256::hex_digest};
        use std::time::Instant;
        const N:usize=21;
        fn median(values:&mut [u128])->u128{values.sort_unstable();values[values.len()/2]}
        let t=Temp::new();let store=ArtifactStore::open(&t.0).unwrap();
        let inputs=package_inputs(MATH);let lock=package_lock(&inputs);let pin=hex_digest(lock.as_bytes());
        let start=Instant::now();
        let mut binary=Vec::new();
        File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut binary).unwrap();
        assert!(binary.len()<=128*1024*1024);
        let compiler=hex_digest(&binary);
        let toolchain=hex_digest(b"fixture: same executable includes complete compiler");
        let identity_ns=start.elapsed().as_nanos();
        let prepare=||witness::prepare("src/main.atto",MANIFEST,&lock,&pin,&inputs,TargetOs::Posix,&compiler,&toolchain).unwrap();
        let prepared=prepare();let baseline=prepared.compile().unwrap();
        store.publish(&prepared.key,&receipt::seal(&prepared.key,&baseline,&SECRET).unwrap(),&SECRET).unwrap();
        let (mut cold,mut warm,mut admission,mut emission,mut retrieval)=(Vec::new(),Vec::new(),Vec::new(),Vec::new(),Vec::new());
        for _ in 0..N{
            let start=Instant::now();let prepared=prepare();
            admission.push(start.elapsed().as_nanos());let step=Instant::now();
            let bytes=prepared.compile().unwrap();emission.push(step.elapsed().as_nanos());
            cold.push(start.elapsed().as_nanos());assert_eq!(bytes,baseline);
            let start=Instant::now();let prepared=prepare();let step=Instant::now();
            let bytes=store.load(&prepared.key,&SECRET).unwrap().unwrap();retrieval.push(step.elapsed().as_nanos());
            warm.push(start.elapsed().as_nanos());assert_eq!(bytes,baseline);
        }
        println!("CACHE_TIMING_FIXTURE|n={N}|identity_once_ns={identity_ns}|admission_median_ns={}|frontend_codegen_median_ns={}|verified_read_median_ns={}|cold_median_ns={}|warm_median_ns={}|bytes={}",
            median(&mut admission),median(&mut emission),median(&mut retrieval),median(&mut cold),median(&mut warm),baseline.len());
        // Smoke measurements only: debug build, tiny fixture, hot OS page cache,
        // excludes process launch, publisher fsync and guest runtime. No threshold.
    }

    fn session_compiler_identity()->String{
        // Read the actual host test image rather than a caller-selected version.
        // Amortized once in each process. This is not a deployment attestation.
        static ID:std::sync::OnceLock<String>=std::sync::OnceLock::new();
        ID.get_or_init(||{
            let mut binary=Vec::new();
            File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut binary).unwrap();
            assert!(binary.len()<=128*1024*1024);
            crate::sha256::hex_digest(&binary)
        }).clone()
    }
    #[test]
    fn explicit_session_pinned_ledger_restart_and_corruption(){
        use crate::{compiler_cache_session::{Session,Mode,Outcome},compiler_cache_ledger as ledger,
            library::TargetOs,sha256::{hex_digest,sha256}};
        let t=Temp::new();
        let cache=t.0.join("cache");fs::DirBuilder::new().mode(0o700).create(&cache).unwrap();
        let host=t.0.join("host");fs::DirBuilder::new().mode(0o700).create(&host).unwrap();
        let store=ArtifactStore::open(&cache).unwrap();
        let authority=authority::Authority::open(&host).unwrap();
        let record=ledger::record("tenant/a",1,SECRET).unwrap();
        let pin=sha256(&record);
        authority.provision("tenant/a",1,SECRET,&pin).unwrap();
        assert!(authority.provision("tenant/a",1,SECRET,&pin).is_err());
        assert!(authority::Authority::open(&cache).unwrap().distinct_from(&store).is_err());
        let compiler=session_compiler_identity();
        let toolchain=hex_digest(b"fixture toolchain attestation");
        let session=Session{store:&store,scope:"tenant/a",compiler_digest:&compiler,toolchain_digest:&toolchain,target:TargetOs::Posix};
        let inputs=package_inputs(MATH);let lock=package_lock(&inputs);let source_pin=hex_digest(lock.as_bytes());
        let build=|mode,pin:&[u8;32],master,inputs:&[crate::package_resolver::SnapshotSource<'_>]|
            session.build(mode,&authority,pin,master,"src/main.atto",MANIFEST,&lock,&source_pin,inputs);
        let cold=build(Mode::Cold,&pin,SECRET,&inputs).unwrap();
        assert_eq!(cold.outcome,Outcome::Compiled);
        let warm=build(Mode::Incremental,&pin,SECRET,&inputs).unwrap();
        assert_eq!(warm.outcome,Outcome::Reused);assert_eq!(cold.bytes,warm.bytes);
        fs::write(host.join("guest.wasm"),&warm.bytes).unwrap();execute_wasm(&host.join("guest.wasm"));
        let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","compiler_artifact_store::tests::explicit_session_child","--nocapture"])
            .env("ATTO_SESSION_ROOT",&t.0).status().unwrap();assert!(status.success());
        let changed=MATH.replace("x * 2","x * 3");
        assert_eq!(build(Mode::Incremental,&pin,SECRET,&package_inputs(&changed)).unwrap_err().code,"E_LOCK_HASH");
        let rotated=ledger::record("tenant/a",2,[7;32]).unwrap();let new_pin=sha256(&rotated);
        let held=authority.lease("tenant/a",&pin,SECRET).unwrap();
        let try_rotate=|expect:&str|Command::new(std::env::current_exe().unwrap()).args(["--exact","compiler_artifact_store::tests::authority_rotation_child","--nocapture"])
            .env("ATTO_ROTATION_ROOT",&host).env("ATTO_ROTATION_EXPECT",expect).status().unwrap();
        assert!(try_rotate("busy").success());
        assert_eq!(fs::read(host.join("active")).unwrap(),record);
        drop(held);
        // Pending file left by interrupted publisher is replaced under lock.
        fs::write(host.join(".ledger.pending"),b"interrupted").unwrap();
        assert!(try_rotate("success").success());
        assert_eq!(fs::read(host.join("active")).unwrap(),rotated);
        assert!(!host.join(".ledger.pending").exists());
        assert_eq!(build(Mode::Incremental,&pin,SECRET,&inputs).unwrap_err().code,"E_CACHE_LEDGER");
        assert_eq!(build(Mode::Incremental,&new_pin,[7;32],&inputs).unwrap_err().code,"E_CACHE_READ");
        let fresh=build(Mode::Cold,&new_pin,[7;32],&inputs).unwrap();
        assert_eq!(fresh.outcome,Outcome::Compiled);assert_eq!(fresh.bytes,cold.bytes);
        assert_eq!(build(Mode::Incremental,&new_pin,[7;32],&inputs).unwrap().outcome,Outcome::Reused);
        let prepared=crate::compiler_cache_witness::prepare("src/main.atto",MANIFEST,&lock,&source_pin,&inputs,TargetOs::Posix,&compiler,&toolchain).unwrap();
        let path=cache.join(slot(&prepared.key));let mut corrupted=fs::read(&path).unwrap();corrupted[112]^=1;
        fs::write(&path,&corrupted).unwrap();
        assert_eq!(build(Mode::Incremental,&new_pin,[7;32],&inputs).unwrap_err().code,"E_CACHE_READ");
        assert_eq!(fs::read(&path).unwrap(),corrupted);
        // Restore old record on disk but retain independently supplied pin.
        fs::write(host.join("active"),&record).unwrap();
        assert_eq!(build(Mode::Incremental,&new_pin,[7;32],&inputs).unwrap_err().code,"E_CACHE_LEDGER");
        fs::write(host.join("active"),&rotated).unwrap();
        // Explicit recovery of the deliberately corrupted artifact.
        assert_eq!(build(Mode::Cold,&new_pin,[7;32],&inputs).unwrap().outcome,Outcome::Compiled);
        let changed_inputs=package_inputs(&changed);let changed_lock=package_lock(&changed_inputs);
        let changed_pin=hex_digest(changed_lock.as_bytes());
        let changed_build=session.build(Mode::Incremental,&authority,&new_pin,[7;32],"src/main.atto",MANIFEST,&changed_lock,&changed_pin,&changed_inputs).unwrap();
        assert_eq!(changed_build.outcome,Outcome::Compiled);assert_ne!(changed_build.bytes,cold.bytes);
        let expected=crate::package_resolver::compile_locked_snapshot("src/main.atto",MANIFEST,&changed_lock,&changed_pin,&changed_inputs).unwrap();
        assert_eq!(expected,changed_build.bytes);
        fs::write(host.join("changed.wasm"),&expected).unwrap();execute_wasm_result(&host.join("changed.wasm"),"63");
        assert_eq!(session.build(Mode::Incremental,&authority,&new_pin,[7;32],"src/main.atto",MANIFEST,&changed_lock,&changed_pin,&changed_inputs).unwrap().outcome,Outcome::Reused);
        // Independently pinned, syntactically valid but ill-typed source must
        // never publish or reuse an executable. The clean diagnostic agrees.
        let bad=MATH.replace("return x * 2;","return true;");
        let bad_inputs=package_inputs(&bad);let bad_lock=package_lock(&bad_inputs);let bad_pin=hex_digest(bad_lock.as_bytes());
        let clean=crate::package_resolver::compile_locked_snapshot("src/main.atto",MANIFEST,&bad_lock,&bad_pin,&bad_inputs).unwrap_err();
        let before:std::collections::BTreeMap<_,_>=fs::read_dir(&cache).unwrap().map(|e|{let e=e.unwrap();(e.file_name(),fs::read(e.path()).unwrap())}).collect();
        let actual=session.build(Mode::Cold,&authority,&new_pin,[7;32],"src/main.atto",MANIFEST,&bad_lock,&bad_pin,&bad_inputs).unwrap_err();
        assert_eq!(actual.code,clean.code);
        let after:std::collections::BTreeMap<_,_>=fs::read_dir(&cache).unwrap().map(|e|{let e=e.unwrap();(e.file_name(),fs::read(e.path()).unwrap())}).collect();
        assert_eq!(before,after);
        println!("CACHE_SESSION|cross-process-rotation-fenced|restart|rollback-pin-denied|changed-guest=63|failed-build-no-publication");
    }
    #[test]
    fn explicit_session_child(){
        if let Some(root)=std::env::var_os("ATTO_SESSION_ROOT"){
            use crate::{compiler_cache_session::{Session,Mode,Outcome},compiler_cache_ledger as ledger,
                library::TargetOs,sha256::{hex_digest,sha256}};
            let root=Path::new(&root);let store=ArtifactStore::open(&root.join("cache")).unwrap();
            let authority=authority::Authority::open(&root.join("host")).unwrap();
            let expected=ledger::record("tenant/a",1,SECRET).unwrap();let pin=sha256(&expected);
            let compiler=session_compiler_identity();let toolchain=hex_digest(b"fixture toolchain attestation");
            let session=Session{store:&store,scope:"tenant/a",compiler_digest:&compiler,toolchain_digest:&toolchain,target:TargetOs::Posix};
            let inputs=package_inputs(MATH);let lock=package_lock(&inputs);let source_pin=hex_digest(lock.as_bytes());
            let warm=session.build(Mode::Incremental,&authority,&pin,SECRET,"src/main.atto",MANIFEST,&lock,&source_pin,&inputs).unwrap();
            assert_eq!(warm.outcome,Outcome::Reused);
            assert_eq!(warm.bytes,fs::read(root.join("host/guest.wasm")).unwrap());
            println!("CACHE_SESSION_CHILD|ledger-admitted|reused");
        }
    }
    #[test]
    fn authority_rotation_child(){
        if let Some(root)=std::env::var_os("ATTO_ROTATION_ROOT"){
            use crate::{compiler_cache_ledger as ledger,sha256::sha256};
            let authority=authority::Authority::open(Path::new(&root)).unwrap();
            let old=sha256(&ledger::record("tenant/a",1,SECRET).unwrap());
            let new=sha256(&ledger::record("tenant/a",2,[7;32]).unwrap());
            let result=authority.rotate("tenant/a",&old,SECRET,2,[7;32],&new);
            if std::env::var("ATTO_ROTATION_EXPECT").unwrap()=="busy"{
                assert_eq!(result.unwrap_err().kind(),io::ErrorKind::WouldBlock);
                println!("CACHE_AUTHORITY_CHILD|rotation-blocked-by-live-lease");
            }else{
                result.unwrap();
                assert!(authority.lease("tenant/a",&old,SECRET).is_err());
                assert!(authority.lease("tenant/a",&new,[7;32]).is_ok());
                println!("CACHE_AUTHORITY_CHILD|rotation-committed");
            }
        }
    }
    #[test]
    fn authority_rejects_bad_inode_and_rotation_cas(){
        use crate::{compiler_cache_ledger as ledger,sha256::sha256};
        let t=Temp::new();let a=authority::Authority::open(&t.0).unwrap();
        let old=ledger::record("tenant/a",1,SECRET).unwrap();let pin=sha256(&old);
        let new=sha256(&ledger::record("tenant/a",2,[7;32]).unwrap());
        assert!(a.provision("tenant/a",1,SECRET,&[0;32]).is_err());
        a.provision("tenant/a",1,SECRET,&pin).unwrap();
        assert!(a.rotate("tenant/a",&[0;32],SECRET,2,[7;32],&new).is_err());
        assert!(a.rotate("tenant/a",&pin,SECRET,2,[7;32],&[0;32]).is_err());
        assert!(a.rotate("tenant/a",&pin,SECRET,1,[7;32],&new).is_err());
        assert!(a.rotate("tenant/a",&pin,SECRET,2,SECRET,&new).is_err());
        assert_eq!(fs::read(t.0.join("active")).unwrap(),old);
        let path=t.0.join("active");fs::rename(&path,t.0.join("saved")).unwrap();
        symlink(t.0.join("saved"),&path).unwrap();assert!(a.lease("tenant/a",&pin,SECRET).is_err());fs::remove_file(&path).unwrap();
        fs::hard_link(t.0.join("saved"),&path).unwrap();assert!(a.lease("tenant/a",&pin,SECRET).is_err());fs::remove_file(&path).unwrap();
        assert!(Command::new("mkfifo").arg(&path).status().unwrap().success());assert!(a.lease("tenant/a",&pin,SECRET).is_err());fs::remove_file(&path).unwrap();
        fs::rename(t.0.join("saved"),&path).unwrap();assert!(a.lease("tenant/a",&pin,SECRET).is_ok());
        println!("CACHE_AUTHORITY|symlink-hardlink-fifo-denied|bad-cas-no-mutation");
    }

    #[test]
    fn twelve_source_locked_package_differential(){
        use crate::{compiler_cache_session::{Session,Mode,Outcome},compiler_cache_ledger as ledger,
            library::TargetOs,sha256::{hex_digest,sha256},package_resolver::SnapshotSource};
        let t=Temp::new();let cache=t.0.join("cache");let host=t.0.join("host");
        for dir in [&cache,&host]{fs::DirBuilder::new().mode(0o700).create(dir).unwrap();}
        let store=ArtifactStore::open(&cache).unwrap();
        let authority=authority::Authority::open(&host).unwrap();
        let pin=sha256(&ledger::record("tenant/a",1,SECRET).unwrap());
        authority.provision("tenant/a",1,SECRET,&pin).unwrap();
        let compiler=session_compiler_identity();let toolchain=hex_digest(b"fixture toolchain attestation");
        let session=Session{store:&store,scope:"tenant/a",compiler_digest:&compiler,toolchain_digest:&toolchain,target:TargetOs::Posix};
        let inputs=[
            SnapshotSource{path:"src/main.atto",source:include_str!("../tests/fixtures/incremental_locked/src/main.atto")},
            SnapshotSource{path:"lib/m00.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m00.atto")},
            SnapshotSource{path:"lib/m01.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m01.atto")},
            SnapshotSource{path:"lib/m02.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m02.atto")},
            SnapshotSource{path:"lib/m03.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m03.atto")},
            SnapshotSource{path:"lib/m04.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m04.atto")},
            SnapshotSource{path:"lib/m05.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m05.atto")},
            SnapshotSource{path:"lib/m06.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m06.atto")},
            SnapshotSource{path:"lib/m07.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m07.atto")},
            SnapshotSource{path:"lib/m08.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m08.atto")},
            SnapshotSource{path:"lib/m09.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m09.atto")},
            SnapshotSource{path:"lib/m10.atto",source:include_str!("../tests/fixtures/incremental_locked/lib/m10.atto")},
        ];
        let lock=package_lock(&inputs);let source_pin=hex_digest(lock.as_bytes());
        let mut memo=crate::package_resolver::SnapshotMemo::default();
        let mut build=|mode,lock:&str,source_pin:&str,inputs:&[SnapshotSource<'_>]|session.build_memoized(mode,&authority,&pin,SECRET,"src/main.atto",MANIFEST,lock,source_pin,inputs,&mut memo);
        let cold=build(Mode::Cold,&lock,&source_pin,&inputs).unwrap();
        assert_eq!(cold.outcome,Outcome::Compiled);
        assert_eq!((cold.source_work.rewritten,cold.source_work.reused),(12,0));
        let warm=build(Mode::Incremental,&lock,&source_pin,&inputs).unwrap();
        assert_eq!(warm.outcome,Outcome::Reused);assert_eq!(warm.bytes,cold.bytes);
        assert_eq!((warm.source_work.rewritten,warm.source_work.reused),(0,12));
        let path=host.join("fixture.wasm");fs::write(&path,&warm.bytes).unwrap();execute_wasm(&path);
        let edited=inputs[11].source.replace("return 2;","return 3;");
        let mut changed=inputs;changed[11].source=&edited;
        assert_eq!(build(Mode::Incremental,&lock,&source_pin,&changed).unwrap_err().code,"E_LOCK_HASH");
        let updated_lock=package_lock(&changed);let updated_pin=hex_digest(updated_lock.as_bytes());
        let next=build(Mode::Incremental,&updated_lock,&updated_pin,&changed).unwrap();
        assert_eq!(next.outcome,Outcome::Compiled);
        assert_eq!((next.source_work.rewritten,next.source_work.reused),(1,11));
        assert_eq!(next.bytes,crate::package_resolver::compile_locked_snapshot("src/main.atto",MANIFEST,&updated_lock,&updated_pin,&changed).unwrap());
        fs::write(&path,&next.bytes).unwrap();execute_wasm_result(&path,"43");
        let hot=build(Mode::Incremental,&updated_lock,&updated_pin,&changed).unwrap();
        assert_eq!(hot.outcome,Outcome::Reused);
        assert_eq!((hot.source_work.rewritten,hot.source_work.reused),(0,12));
        let invalid=edited.replace("return 3;","return true;");
        let mut bad=changed;bad[11].source=&invalid;
        let bad_lock=package_lock(&bad);let bad_pin=hex_digest(bad_lock.as_bytes());
        let clean_error=crate::package_resolver::compile_locked_snapshot("src/main.atto",MANIFEST,&bad_lock,&bad_pin,&bad).unwrap_err();
        assert_eq!(build(Mode::Incremental,&bad_lock,&bad_pin,&bad).unwrap_err().code,clean_error.code);
        let after=build(Mode::Incremental,&updated_lock,&updated_pin,&changed).unwrap();
        assert_eq!(after.bytes,next.bytes);assert_eq!((after.source_work.rewritten,after.source_work.reused),(0,12));
        // Actual per-file source rewrite/import-path memo only. Full linker
        // admission and semantic checking/codegen on a miss are unchanged.
        println!("CACHE_TWELVE_SOURCE|cold=12/0|warm=0/12|edit=1/11|failed-edit-rollback|guest=42/43|source-transform-only");
    }
}
