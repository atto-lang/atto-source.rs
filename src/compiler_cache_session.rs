//! Explicit test-only incremental interface, no production CLI or API.
//! Each build re-admits the externally pinned active epoch and authenticated
//! source closure. No fallback on corruption, no caller-supplied signing bytes.
//! The host must guarantee that the current record/pin is supplied at every
//! admission and serialize rotation with builds; this candidate has no live
//! authority service or durable anti-rollback provider.
use crate::{compiler_artifact_store::ArtifactStore,compiler_artifact_receipt as receipt,
    compiler_cache_ledger as ledger,compiler_cache_witness as witness,
    diag::{Diagnostic,Pos},library::TargetOs,package_resolver::SnapshotSource};

pub(crate) enum Mode { Cold, Incremental }
#[derive(Debug,PartialEq,Eq)]
pub(crate) enum Outcome { Compiled, Reused }
#[derive(Debug)]
pub(crate) struct Build { pub(crate) bytes:Vec<u8>, pub(crate) outcome:Outcome }
fn cache_error(code:&'static str)->Diagnostic{Diagnostic::new(Pos::default(),code,"cache admission or storage failed")}
pub(crate) struct Session<'a>{
    pub(crate) store:&'a ArtifactStore,
    pub(crate) scope:&'a str,
    pub(crate) compiler_digest:&'a str,
    pub(crate) toolchain_digest:&'a str,
    pub(crate) target:TargetOs,
}
impl Session<'_>{
    pub(crate) fn build(&self,mode:Mode,record:&[u8],current_pin:&[u8;32],master:[u8;32],
        entry:&str,manifest:&str,lock:&str,source_pin:&str,inputs:&[SnapshotSource<'_>])->Result<Build,Diagnostic>{
        let active=ledger::admit(self.scope,record,current_pin,master).map_err(|_|cache_error("E_CACHE_LEDGER"))?;
        let secret=active.derived();
        let prepared=witness::prepare(entry,manifest,lock,source_pin,inputs,self.target,self.compiler_digest,self.toolchain_digest)?;
        if matches!(mode,Mode::Incremental){
            if let Some(bytes)=self.store.load(&prepared.key,&secret).map_err(|_|cache_error("E_CACHE_READ"))?{
                return Ok(Build{bytes,outcome:Outcome::Reused});
            }
        }
        let bytes=prepared.compile()?;
        // Only successful compiler output can reach signing/publication.
        let sealed=receipt::seal(&prepared.key,&bytes,&secret).map_err(|_|cache_error("E_CACHE_RECEIPT"))?;
        self.store.publish(&prepared.key,&sealed,&secret).map_err(|_|cache_error("E_CACHE_PUBLISH"))?;
        Ok(Build{bytes,outcome:Outcome::Compiled})
    }
}
