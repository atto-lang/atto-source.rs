//! Explicit opt-in incremental compilation for trusted Linux x86-64 hosts.
//! EXPERIMENTAL: not a production security boundary or general language profile.
//! Host must independently attest build closure and current authority pin,
//! protect separate directories/ancestors, provision random master keys and
//! coordinate external epoch changes. No runtime guest access or signing
//! service. Do not expose this API directly to untrusted tenants.
//! No external registry package linking, per-unit typed-IR or codegen reuse.
//! Cold mode compiles AND publishes. Default compiler APIs remain unchanged.
use crate::{compiler_artifact_store::{ArtifactStore,authority::Authority},
    compiler_cache_ledger as ledger,compiler_cache_session as session,
    package_resolver::{SnapshotMemo,SnapshotSource},library::TargetOs,
    diag::Diagnostic};
use std::{io,path::Path};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Mode { Cold, Incremental }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Outcome { Compiled, Reused }
#[derive(Debug)]
pub struct Build {
    pub bytes:Vec<u8>,
    pub outcome:Outcome,
    /// Actual source-local rewrite/import-path work; NOT typecheck counts.
    pub rewritten_sources:usize,
    pub reused_source_transforms:usize,
}
pub struct Snapshot<'a> {
    pub entry:&'a str,
    pub manifest:&'a str,
    pub lockfile:&'a str,
    pub trusted_lock_sha256:&'a str,
    pub sources:&'a [SnapshotSource<'a>],
}
/// Host-attested immutable build identities. A claimed version is insufficient.
pub struct BuildIdentity<'a> {
    pub compiler_sha256:&'a str,
    pub toolchain_sha256:&'a str,
    pub target:TargetOs,
}
pub struct HostSession {
    cache:ArtifactStore,
    authority:Authority,
    scope:String,
    compiler:String,
    toolchain:String,
    target:TargetOs,
    memo:SnapshotMemo,
}
fn valid_digest(s:&str)->bool{
    s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
}
fn invalid()->io::Error{io::Error::new(io::ErrorKind::InvalidInput,"invalid experimental host configuration")}
impl HostSession {
    /// Both directories must exist, be owner-only and independently protected.
    /// Host must maintain the independently current pin; this adapter does
    /// not supply a rollback-resistant external ledger or key vault.
    pub fn open(cache:&Path,authority:&Path,scope:&str,identity:BuildIdentity<'_>)->io::Result<Self>{
        if scope.is_empty()||scope.len()>512||!scope.bytes().all(|b|(33..=126).contains(&b))||
            !valid_digest(identity.compiler_sha256)||!valid_digest(identity.toolchain_sha256){return Err(invalid());}
        let cache=ArtifactStore::open(cache)?;
        let authority=Authority::open(authority)?;
        authority.distinct_from(&cache)?;
        Ok(Self{cache,authority,scope:scope.to_owned(),compiler:identity.compiler_sha256.to_owned(),
            toolchain:identity.toolchain_sha256.to_owned(),target:identity.target,memo:SnapshotMemo::default()})
    }
    /// Deterministic non-secret record for external host approval.
    /// Never derive the trusted current pin from a record read out of cache.
    pub fn proposed_record(&self,epoch:u64,master:[u8;32])->io::Result<Vec<u8>>{
        ledger::record(&self.scope,epoch,master).map_err(|_|invalid())
    }
    pub fn provision(&self,epoch:u64,master:[u8;32],approved_pin:&[u8;32])->io::Result<()>{
        self.authority.provision(&self.scope,epoch,master,approved_pin)
    }
    pub fn rotate(&mut self,old_pin:&[u8;32],old_master:[u8;32],next_epoch:u64,next_master:[u8;32],
        approved_next_pin:&[u8;32])->io::Result<()>{
        // Clear source memo even on an indeterminate durability result.
        self.memo=SnapshotMemo::default();
        self.authority.rotate(&self.scope,old_pin,old_master,next_epoch,next_master,approved_next_pin)
    }
    pub fn build(&mut self,mode:Mode,current_pin:&[u8;32],master:[u8;32],snapshot:Snapshot<'_>)->Result<Build,Diagnostic>{
        let session=session::Session{store:&self.cache,scope:&self.scope,compiler_digest:&self.compiler,
            toolchain_digest:&self.toolchain,target:self.target};
        let result=session.build_memoized(match mode{Mode::Cold=>session::Mode::Cold,Mode::Incremental=>session::Mode::Incremental},
            &self.authority,current_pin,master,snapshot.entry,snapshot.manifest,snapshot.lockfile,
            snapshot.trusted_lock_sha256,snapshot.sources,&mut self.memo)?;
        Ok(Build{bytes:result.bytes,outcome:match result.outcome{session::Outcome::Compiled=>Outcome::Compiled,
            session::Outcome::Reused=>Outcome::Reused},rewritten_sources:result.source_work.rewritten,
            reused_source_transforms:result.source_work.reused})
    }
}
