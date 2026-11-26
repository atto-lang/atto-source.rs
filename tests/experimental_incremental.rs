#![cfg(all(feature = "experimental-incremental-cache", target_os = "linux", target_arch = "x86_64"))]
use atto_compiler::{experimental_incremental::{HostSession,BuildIdentity,Snapshot,Mode,Outcome},
    package_resolver::{SnapshotSource,compile_locked_snapshot},library::TargetOs,sha256::{sha256,hex_digest}};
use std::{fs,io::{Read,Write},os::unix::fs::DirBuilderExt,path::PathBuf,process::{Command,Stdio}};
struct Temp(PathBuf);
impl Drop for Temp{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
const MANIFEST:&str="[package]\nname=\"api-fixture\"\nversion=\"1.0.0\"\n";
const MASTER:[u8;32]=[7;32]; // Test only.
fn lock(source:&str)->String{format!("[lock]\nmanifest_sha256=\"{}\"\n[files]\n\"main.atto\"=\"{}\"\n",hex_digest(MANIFEST.as_bytes()),hex_digest(source.as_bytes()))}
#[test]
fn explicit_host_api_compiles_and_reuses_from_a_separate_library_client(){
    let root=Temp(std::env::temp_dir().join(format!("atto-host-api-{}",std::process::id())));
    fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
    let cache=root.0.join("cache");let authority=root.0.join("authority");
    for dir in [&cache,&authority]{fs::DirBuilder::new().mode(0o700).create(dir).unwrap();}
    let mut exe=Vec::new();fs::File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut exe).unwrap();
    assert!(exe.len()<=128*1024*1024);
    let compiler=hex_digest(&exe);let toolchain=hex_digest(b"test binary statically linked compiler fixture");
    let identity=||BuildIdentity{compiler_sha256:&compiler,toolchain_sha256:&toolchain,target:TargetOs::Posix};
    assert!(HostSession::open(&cache,&cache,"tenant/a",identity()).is_err());
    let open=||HostSession::open(&cache,&authority,"tenant/a",identity()).unwrap();
    let mut session=open();
    let pin=sha256(&session.proposed_record(1,MASTER).unwrap());
    session.provision(1,MASTER,&pin).unwrap();
    let source="pub fnc main(): int { return 42; }";
    let sources=[SnapshotSource{path:"main.atto",source}];let lockfile=lock(source);let trusted=hex_digest(lockfile.as_bytes());
    let snapshot=||Snapshot{entry:"main.atto",manifest:MANIFEST,lockfile:&lockfile,trusted_lock_sha256:&trusted,sources:&sources};
    let cold=session.build(Mode::Cold,&pin,MASTER,snapshot()).unwrap();
    assert_eq!(cold.outcome,Outcome::Compiled);assert_eq!(cold.rewritten_sources,1);
    assert_eq!(cold.bytes,compile_locked_snapshot("main.atto",MANIFEST,&lockfile,&trusted,&sources).unwrap());
    let warm=session.build(Mode::Incremental,&pin,MASTER,snapshot()).unwrap();
    assert_eq!(warm.outcome,Outcome::Reused);assert_eq!(warm.reused_source_transforms,1);assert_eq!(warm.bytes,cold.bytes);
    drop(session);
    let mut reopened=open();let warm=reopened.build(Mode::Incremental,&pin,MASTER,snapshot()).unwrap();
    assert_eq!(warm.outcome,Outcome::Reused);assert_eq!(warm.rewritten_sources,1);assert_eq!(warm.bytes,cold.bytes);
    let mut child=Command::new("node").args(["-e","const a=require('node:assert/strict'),b=require('node:fs').readFileSync(0);a.ok(WebAssembly.validate(b));const m=new WebAssembly.Module(b);a.deepEqual(WebAssembly.Module.imports(m),[]);const i=new WebAssembly.Instance(m,{});a.equal(i.exports.main(),42n);a.equal(i.exports.panic_code.value,0);"]).stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&warm.bytes).unwrap();assert!(child.wait().unwrap().success());
    assert_eq!(reopened.build(Mode::Incremental,&[0;32],MASTER,snapshot()).unwrap_err().code,"E_CACHE_LEDGER");
    println!("CACHE_EXPERIMENTAL_HOST_API|feature-enabled|real-guest=42|explicit-cold-warm|reopened|stale-pin-denied");
}
