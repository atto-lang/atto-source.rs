//! Internal bounded locked-package stage witness. No production cache activation.
//! Host must independently attest the compiler/toolchain build identities.
//! Single-package immutable input only; no external dependency registry.
use crate::{compiler_cache_key as key,diag::{Diagnostic,Pos},library::{self,TargetOs},
    package_resolver::{self,SnapshotSource},sha256::hex_digest};

pub(crate) struct Prepared {
    pub(crate) key: [u8;32],
    linked: String,
    target: TargetOs,
    pub(crate) memo: package_resolver::SnapshotMemo,
    pub(crate) work: package_resolver::SnapshotWork,
}
fn error(detail:&str)->Diagnostic{Diagnostic::new(Pos::default(),"E_CACHE_KEY",detail)}
pub(crate) fn prepare(entry:&str,manifest:&str,lock:&str,pin:&str,inputs:&[SnapshotSource<'_>],
    target:TargetOs,compiler_digest:&str,toolchain_digest:&str)->Result<Prepared,Diagnostic>{
    prepare_memoized(entry,manifest,lock,pin,inputs,target,compiler_digest,toolchain_digest,&package_resolver::SnapshotMemo::default())
}
pub(crate) fn prepare_memoized(entry:&str,manifest:&str,lock:&str,pin:&str,inputs:&[SnapshotSource<'_>],
    target:TargetOs,compiler_digest:&str,toolchain_digest:&str,previous:&package_resolver::SnapshotMemo)->Result<Prepared,Diagnostic>{
    library::with_target_os(target,|| {
        let graph=package_resolver::prepare_locked_snapshot_memoized(entry,manifest,lock,pin,inputs,previous)?;
        let target_digest=hex_digest(format!("atto-wasm-stage-v1|wasm32|abi={}|os={}",
            crate::ABI_VERSION,match target{TargetOs::Posix=>"posix",TargetOs::Windows=>"windows"}).as_bytes());
        let configuration=hex_digest(b"locked-snapshot-v1|single-package|no-external-host-grants|ordinary-compiler");
        let bindings=hex_digest(graph.linked.as_bytes());
        let encoded=key::encode(&key::Input{stage:"wasm-module",owner:&graph.identity,unit_kind:"source-unit",
            unit:entry,compiler:compiler_digest,toolchain:toolchain_digest,target:&target_digest,
            configuration:&configuration,sources:pin,bindings:&bindings,generated:None,dependencies:&[]})
            .map_err(error)?;
        let mut bytes=[0u8;32];
        for (i,b) in bytes.iter_mut().enumerate(){
            *b=u8::from_str_radix(&encoded[i*2..i*2+2],16).map_err(|_|error("invalid encoded key"))?;
        }
        Ok(Prepared{key:bytes,linked:graph.linked,target,memo:graph.memo,work:graph.work})
    })
}
impl Prepared {
    pub(crate) fn compile(&self)->Result<Vec<u8>,Diagnostic>{
        library::with_target_os(self.target,||crate::compile(&self.linked))
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn target_scopes_are_nested_thread_local_and_unwind_safe(){
        let default=library::target_os();
        let other=if default==TargetOs::Posix{TargetOs::Windows}else{TargetOs::Posix};
        library::with_target_os(other,||{
            assert_eq!(library::target_os(),other);
            assert_eq!(std::thread::spawn(library::target_os).join().unwrap(),default);
            library::with_target_os(default,||assert_eq!(library::target_os(),default));
            let result=std::panic::catch_unwind(||library::with_target_os(default,||panic!("fixture unwind")));
            assert!(result.is_err());
            assert_eq!(library::target_os(),other);
        });
        assert_eq!(library::target_os(),default);
    }

    #[test]
    fn target_specific_library_bytes_are_distinct_and_executable(){
        let source=r#"import module path from "atto.std/path"; pub fnc main(): int { return path.separator_code(); }"#;
        let posix=library::with_target_os(TargetOs::Posix,||crate::compile(source)).unwrap();
        let windows=library::with_target_os(TargetOs::Windows,||crate::compile(source)).unwrap();
        assert_ne!(posix,windows);
        let execute=|bytes:&[u8],expected:&str|{
            use std::io::Write;
            use std::process::{Command,Stdio};
            let script="const fs=require('node:fs'),assert=require('node:assert/strict');const b=fs.readFileSync(0);assert.ok(WebAssembly.validate(b));const m=new WebAssembly.Module(b);assert.deepEqual(WebAssembly.Module.imports(m),[]);const i=new WebAssembly.Instance(m,{});assert.equal(i.exports.main(),BigInt(process.argv[1]));assert.equal(i.exports.panic_code.value,0);console.log('CACHE_TARGET_GUEST|result='+process.argv[1])";
            let mut child=Command::new("node").args(["-e",script,expected]).stdin(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(bytes).unwrap();
            assert!(child.wait().unwrap().success());
        };
        execute(&posix,"47");execute(&windows,"92");
        assert_eq!(posix,library::with_target_os(TargetOs::Posix,||crate::compile(source)).unwrap());
    }
}
