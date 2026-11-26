//! Explicit opt-in incremental compilation for trusted Linux x86-64 hosts.
//! EXPERIMENTAL: not a production security boundary or general language profile.
//! Host must independently attest build closure and current authority pin,
//! protect separate directories/ancestors, provision random master keys and
//! coordinate external epoch changes. No runtime guest access or signing
//! service. Do not expose this API directly to untrusted tenants.
//! Single-file locked application roots may explicitly supply a host-approved
//! set of frozen registry dependencies. No transitive registry acquisition,
//! per-unit typed-IR or codegen reuse.
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
/// Independently authenticated dependency snapshot, approved by the host.
/// Supplying a snapshot authorizes this name in the bounded application
/// profile; no ambient registry lookup and no transitive resolution.
pub struct RegistryDependency<'a> {
    pub name:&'a str,
    pub manifest:&'a str,
    pub lockfile:&'a str,
    pub trusted_lock_sha256:&'a str,
    pub sources:&'a [SnapshotSource<'a>],
}
#[derive(Debug)]
pub struct RegistryBuild {
    pub bytes:Vec<u8>,
    pub outcome:Outcome,
    /// Number of provider snapshots freshly authenticated, not reused IR.
    pub verified_dependencies:usize,
    /// Actual immutable registry-view reconstruction/reuse counters.
    /// These are not counts of independently compiled typed IR or Wasm units.
    pub rebuilt_provider_views:usize,
    pub reused_provider_views:usize,
    /// Actual isolated closed-function namespace checks (not final root checks).
    pub checked_namespaces:usize,
    pub reused_namespace_checks:usize,
    /// Changed checked units with equal/different partial callable signatures.
    /// Reporting only: never permits skipped checking of a changed unit.
    pub unchanged_callable_signatures:usize,
    pub changed_callable_signatures:usize,
    pub unknown_callable_signatures:usize,
}
pub struct HostSession {
    cache:ArtifactStore,
    authority:Authority,
    scope:String,
    compiler:String,
    toolchain:String,
    target:TargetOs,
    memo:SnapshotMemo,
    registry_memo:crate::compiler_cache_registry::RegistryMemo,
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
            toolchain:identity.toolchain_sha256.to_owned(),target:identity.target,memo:SnapshotMemo::default(),
            registry_memo:crate::compiler_cache_registry::RegistryMemo::default()})
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
        self.registry_memo=crate::compiler_cache_registry::RegistryMemo::default();
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

    /// Experimental whole-artifact reuse for one locked application file
    /// and an explicit independently pinned external provider set.
    pub fn build_registry(&mut self,mode:Mode,current_pin:&[u8;32],master:[u8;32],
        snapshot:Snapshot<'_>,dependencies:&[RegistryDependency<'_>])->Result<RegistryBuild,Diagnostic>{
        use crate::{compiler_cache_registry as registry,compiler_artifact_receipt as receipt,diag::Pos};
        let err=|code|Diagnostic::new(Pos::default(),code,"registry cache admission or storage failed");
        // Isolated-check memoization also changes evidence side effects.
        // Require a separate real clean compilation in evidence mode.
        if crate::borrow_export::enabled(){return Err(err("E_CACHE_EVIDENCE_MODE"));}
        let lease=self.authority.lease(&self.scope,current_pin,master).map_err(|_|err("E_CACHE_LEDGER"))?;
        let secret=lease.secret();
        let empty=registry::RegistryMemo::default();
        let previous=if mode==Mode::Cold{&empty}else{&self.registry_memo};
        let prepared=registry::prepare(snapshot,dependencies,self.target,&self.compiler,&self.toolchain,previous)?;
        if mode==Mode::Incremental {
            if let Some(bytes)=self.cache.load(&prepared.key,&secret).map_err(|_|err("E_CACHE_READ"))?{
                let work=prepared.work;
                self.registry_memo=prepared.memo;
                return Ok(RegistryBuild{bytes,outcome:Outcome::Reused,verified_dependencies:dependencies.len(),
                    rebuilt_provider_views:work.rebuilt,reused_provider_views:work.reused,
                    checked_namespaces:work.semantic.checked,reused_namespace_checks:work.semantic.reused,
                    unchanged_callable_signatures:work.semantic.same_signature,changed_callable_signatures:work.semantic.changed_signature,
                    unknown_callable_signatures:work.semantic.unknown_signature});
            }
        }
        let key=prepared.key;
        let (bytes,memo,work)=prepared.compile()?;
        let sealed=receipt::seal(&key,&bytes,&secret).map_err(|_|err("E_CACHE_RECEIPT"))?;
        self.cache.publish(&key,&sealed,&secret).map_err(|_|err("E_CACHE_PUBLISH"))?;
        self.registry_memo=memo;
        Ok(RegistryBuild{bytes,outcome:Outcome::Compiled,verified_dependencies:dependencies.len(),
            rebuilt_provider_views:work.rebuilt,reused_provider_views:work.reused,
                    checked_namespaces:work.semantic.checked,reused_namespace_checks:work.semantic.reused,
                    unchanged_callable_signatures:work.semantic.same_signature,changed_callable_signatures:work.semantic.changed_signature,
                    unknown_callable_signatures:work.semantic.unknown_signature})
    }
}
