//! Explicit test-only incremental interface, no production CLI or API.
//! Each build re-admits the externally pinned active epoch and authenticated
//! source closure. No fallback on corruption, no caller-supplied signing bytes.
//! The local authority adapter holds a shared lease through publication.
//! The host must supply the independently current pin at every admission;
//! there is no durable external anti-rollback provider.
use crate::{compiler_artifact_store::{ArtifactStore,authority::Authority},compiler_artifact_receipt as receipt,
    compiler_cache_witness as witness,
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
    pub(crate) fn build(&self,mode:Mode,authority:&Authority,current_pin:&[u8;32],master:[u8;32],
        entry:&str,manifest:&str,lock:&str,source_pin:&str,inputs:&[SnapshotSource<'_>])->Result<Build,Diagnostic>{
        authority.distinct_from(self.store).map_err(|_|cache_error("E_CACHE_LEDGER"))?;
        let lease=authority.lease(self.scope,current_pin,master).map_err(|_|cache_error("E_CACHE_LEDGER"))?;
        let secret=lease.secret();
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
