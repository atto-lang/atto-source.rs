//! Internal bounded locked-package stage witness. No production cache activation.
//! Host must independently attest the compiler/toolchain build identities.
//! Single-package immutable input only; no external dependency registry.
use crate::{compiler_cache_key as key,diag::{Diagnostic,Pos},library::{self,TargetOs},
    package_resolver::{self,SnapshotSource},sha256::hex_digest};

pub(crate) struct Prepared {
    pub(crate) key: [u8;32],
    linked: String,
    target: TargetOs,
}
fn error(detail:&str)->Diagnostic{Diagnostic::new(Pos::default(),"E_CACHE_KEY",detail)}
pub(crate) fn prepare(entry:&str,manifest:&str,lock:&str,pin:&str,inputs:&[SnapshotSource<'_>],
    target:TargetOs,compiler_digest:&str,toolchain_digest:&str)->Result<Prepared,Diagnostic>{
    library::with_target_os(target,|| {
        let graph=package_resolver::prepare_locked_snapshot(entry,manifest,lock,pin,inputs)?;
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
        Ok(Prepared{key:bytes,linked:graph.linked,target})
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
}
