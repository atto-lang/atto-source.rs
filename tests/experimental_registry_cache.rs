#![cfg(all(feature = "experimental-incremental-cache", target_os = "linux", target_arch = "x86_64"))]
use atto_compiler::{experimental_incremental::{HostSession,BuildIdentity,Snapshot,RegistryDependency,Mode,Outcome},
    package_resolver::SnapshotSource,registry_namespace::{FrozenRegistry,compile_registry_guest},library::TargetOs,sha256::{sha256,hex_digest}};
use std::{fs,io::{Read,Write},os::unix::fs::DirBuilderExt,path::{Path,PathBuf},process::{Command,Stdio}};
struct Temp(PathBuf);
impl Drop for Temp{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
const MASTER:[u8;32]=[7;32];
const ROOT:&str="[package]\nname=\"app\"\nversion=\"1.0.0\"\n";
const PROVIDER:&str="[package]\nname=\"@org/project/provider\"\nversion=\"1.0.0\"\n[namespaces]\nmath=\"src/math/\"\n";
const OTHER:&str="[package]\nname=\"@org/project/other\"\nversion=\"1.0.0\"\n[namespaces]\nmath=\"src/math/\"\n";
const APP:&str="import pkg '@org/project/provider'; import pkg '@org/project/other'; pub fnc main(): int { return provider.math.get(20)+other.math.get(20); }";
const MATH:&str="use namespace math; pub fnc get(int x): int { return x+1; } fnc secret(int x): int { return x; }";
fn lock(manifest:&str,inputs:&[SnapshotSource<'_>])->String{
    let mut s=format!("[lock]\nmanifest_sha256=\"{}\"\n[files]\n",hex_digest(manifest.as_bytes()));
    for i in inputs{s.push_str(&format!("\"{}\"=\"{}\"\n",i.path,hex_digest(i.source.as_bytes())));}
    s
}
fn execute(bytes:&[u8],expected:&str){
    let mut child=Command::new("node").args(["-e","const a=require('node:assert/strict'),b=require('node:fs').readFileSync(0);a.ok(WebAssembly.validate(b));const m=new WebAssembly.Module(b);a.deepEqual(WebAssembly.Module.imports(m),[]);const i=new WebAssembly.Instance(m,{});a.equal(i.exports.main(),BigInt(process.argv[1]));a.equal(i.exports.panic_code.value,0);",expected]).stdin(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();assert!(child.wait().unwrap().success());
}
fn files(dir:&Path)->std::collections::BTreeMap<std::ffi::OsString,Vec<u8>>{
    fs::read_dir(dir).unwrap().map(|e|{let e=e.unwrap();(e.file_name(),fs::read(e.path()).unwrap())}).collect()
}
#[test]
fn locked_registry_application_cold_warm_edit_and_authority_denials(){
    let t=Temp(std::env::temp_dir().join(format!("atto-registry-cache-{}",std::process::id())));
    fs::DirBuilder::new().mode(0o700).create(&t.0).unwrap();
    let cache=t.0.join("cache");let authority=t.0.join("authority");
    for dir in [&cache,&authority]{fs::DirBuilder::new().mode(0o700).create(dir).unwrap();}
    let mut exe=Vec::new();fs::File::open("/proc/self/exe").unwrap().take(128*1024*1024+1).read_to_end(&mut exe).unwrap();assert!(exe.len()<=128*1024*1024);
    let compiler=hex_digest(&exe);let toolchain=hex_digest(b"test binary statically linked compiler fixture");
    let open=||HostSession::open(&cache,&authority,"tenant/a",BuildIdentity{compiler_sha256:&compiler,toolchain_sha256:&toolchain,target:TargetOs::Posix}).unwrap();
    let mut session=open();let pin=sha256(&session.proposed_record(1,MASTER).unwrap());session.provision(1,MASTER,&pin).unwrap();
    let roots=[SnapshotSource{path:"main.atto",source:APP}];let root_lock=lock(ROOT,&roots);let root_pin=hex_digest(root_lock.as_bytes());
    let root=||Snapshot{entry:"main.atto",manifest:ROOT,lockfile:&root_lock,trusted_lock_sha256:&root_pin,sources:&roots};
    let inputs=[SnapshotSource{path:"src/math/main.atto",source:MATH}];
    let p_lock=lock(PROVIDER,&inputs);let p_pin=hex_digest(p_lock.as_bytes());
    let q_lock=lock(OTHER,&inputs);let q_pin=hex_digest(q_lock.as_bytes());
    let p=||RegistryDependency{name:"@org/project/provider",manifest:PROVIDER,lockfile:&p_lock,trusted_lock_sha256:&p_pin,sources:&inputs};
    let q=||RegistryDependency{name:"@org/project/other",manifest:OTHER,lockfile:&q_lock,trusted_lock_sha256:&q_pin,sources:&inputs};
    let cold=session.build_registry(Mode::Cold,&pin,MASTER,root(),&[p(),q()]).unwrap();
    assert_eq!(cold.outcome,Outcome::Compiled);assert_eq!(cold.verified_dependencies,2);execute(&cold.bytes,"42");
    let vp=FrozenRegistry::verify("@org/project/provider",PROVIDER,&p_lock,&p_pin,&inputs).unwrap();
    let vq=FrozenRegistry::verify("@org/project/other",OTHER,&q_lock,&q_pin,&inputs).unwrap();
    assert_eq!(cold.bytes,compile_registry_guest(APP,&[&vp,&vq]).unwrap());
    let warm=session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[q(),p()]).unwrap();
    assert_eq!(warm.outcome,Outcome::Reused);assert_eq!(warm.bytes,cold.bytes);
    drop(session);let mut session=open();
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[p(),q()]).unwrap().outcome,Outcome::Reused);
    let before=files(&cache);
    let changed=MATH.replace("x+1","x+2");let modified=[SnapshotSource{path:"src/math/main.atto",source:&changed}];
    let stale=RegistryDependency{sources:&modified,..p()};
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[stale,q()]).unwrap_err().code,"E_LOCK_HASH");
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[p(),p()]).unwrap_err().code,"E_AMBIGUOUS");
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[q()]).unwrap_err().code,"E_UNBOUND");
    let forged=RegistryDependency{name:"@org/project/forged",..p()};
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[forged,q()]).unwrap_err().code,"E_LOCK_HASH");
    let private=APP.replace("provider.math.get","provider.math.secret");let bad_roots=[SnapshotSource{path:"main.atto",source:&private}];
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,Snapshot{sources:&bad_roots,..root()},&[p(),q()]).unwrap_err().code,"E_LOCK_HASH");
    let bad_lock=lock(ROOT,&bad_roots);let bad_pin=hex_digest(bad_lock.as_bytes());
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,Snapshot{sources:&bad_roots,lockfile:&bad_lock,trusted_lock_sha256:&bad_pin,..root()},&[p(),q()]).unwrap_err().code,"E_PRIVATE");
    assert_eq!(files(&cache),before);
    let updated_lock=lock(PROVIDER,&modified);let updated_pin=hex_digest(updated_lock.as_bytes());
    let updated=||RegistryDependency{sources:&modified,lockfile:&updated_lock,trusted_lock_sha256:&updated_pin,..p()};
    let next=session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[updated(),q()]).unwrap();
    assert_eq!(next.outcome,Outcome::Compiled);execute(&next.bytes,"43");
    let verified=FrozenRegistry::verify("@org/project/provider",PROVIDER,&updated_lock,&updated_pin,&modified).unwrap();
    assert_eq!(next.bytes,compile_registry_guest(APP,&[&verified,&vq]).unwrap());
    assert_eq!(session.build_registry(Mode::Incremental,&pin,MASTER,root(),&[q(),updated()]).unwrap().outcome,Outcome::Reused);
    println!("CACHE_REGISTRY_APP|two-providers|cold-warm=42|repinned-edit=43|order-independent|stale-root-provider-private-duplicate-missing-denied");
}
