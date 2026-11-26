//! Bounded compiler-private registry stage witness. All provider snapshots
//! are freshly authenticated and the actual resolver runs before lookup.
//! This does not claim transitive package dependency authority or typed IR reuse.
use crate::{compiler_cache_key as key,diag::{Diagnostic,Pos},experimental_incremental::{Snapshot,RegistryDependency},
    lexer::{Token,Tok},library::{self,TargetOs},package_resolver,registry_namespace::{self,FrozenRegistry},sha256::hex_digest};
use std::{collections::BTreeMap,sync::Arc};
const AGGREGATE_CAP:usize=4*1024*1024;
fn err(code:&'static str,detail:&str)->Diagnostic{Diagnostic::new(Pos::default(),code,detail)}
/// Only immutable source-authenticated registry views. Current raw source
/// inventory is always reauthenticated, even on a memo hit.
#[derive(Default)]
pub(crate) struct RegistryMemo {
    target:Option<TargetOs>,
    providers:BTreeMap<String,MemoProvider>,
}
struct MemoProvider { pin:String, view:Arc<FrozenRegistry> }
#[derive(Clone,Copy,Default)]
pub(crate) struct Work {pub(crate) rebuilt:usize,pub(crate) reused:usize}
pub(crate) struct Prepared {
    pub(crate) key:[u8;32],
    tokens:Vec<Token>,
    target:TargetOs,
    pub(crate) memo:RegistryMemo,
    pub(crate) work:Work,
}
impl Prepared {
    pub(crate) fn compile(self)->Result<(Vec<u8>,RegistryMemo,Work),Diagnostic>{
        let bytes=library::with_target_os(self.target,||crate::compile_prelexed(self.tokens,None))?;
        Ok((bytes,self.memo,self.work))
    }
}
// Compiler-private framing, bound to the independently attested compiler
// image. Exhaustive token tags, payload lengths and source positions.
fn token_digest(tokens:&[Token])->Result<String,Diagnostic>{
    if tokens.len()>crate::lexer::MAX_TOKENS{return Err(err("E_SOURCE_LIMIT","registry token budget exceeded"));}
    let mut out=b"atto.registry.bound-tokens.v1\0".to_vec();
    out.extend_from_slice(&(tokens.len() as u32).to_be_bytes());
    for t in tokens {
        out.extend_from_slice(&t.pos.line.to_be_bytes());out.extend_from_slice(&t.pos.col.to_be_bytes());
        match &t.tok{
            Tok::Ident(s)|Tok::Str(s)=>{
                out.push(if matches!(&t.tok,Tok::Ident(_)){1}else{2});
                out.extend_from_slice(&(s.len() as u32).to_be_bytes());out.extend_from_slice(s.as_bytes());
            }
            Tok::Int(n)=>{out.push(3);match n{None=>out.push(0),Some(n)=>{out.push(1);out.extend_from_slice(&n.to_be_bytes());}}}
            Tok::Float(n)=>{out.push(4);out.extend_from_slice(&n.to_be_bytes());}
            Tok::LParen=>out.push(5),
            Tok::RParen=>out.push(6),
            Tok::LBrace=>out.push(7),
            Tok::RBrace=>out.push(8),
            Tok::LBracket=>out.push(9),
            Tok::RBracket=>out.push(10),
            Tok::Semi=>out.push(11),
            Tok::Comma=>out.push(12),
            Tok::Colon=>out.push(13),
            Tok::ColonColon=>out.push(14),
            Tok::Arrow=>out.push(15),
            Tok::LeftArrow=>out.push(16),
            Tok::FatArrow=>out.push(17),
            Tok::Dot=>out.push(18),
            Tok::Ellipsis=>out.push(19),
            Tok::Assign=>out.push(20),
            Tok::EqEq=>out.push(21),
            Tok::NotEq=>out.push(22),
            Tok::Lt=>out.push(23),
            Tok::Le=>out.push(24),
            Tok::Gt=>out.push(25),
            Tok::Ge=>out.push(26),
            Tok::Plus=>out.push(27),
            Tok::PlusPlus=>out.push(28),
            Tok::PlusAssign=>out.push(29),
            Tok::Minus=>out.push(30),
            Tok::MinusMinus=>out.push(31),
            Tok::Star=>out.push(32),
            Tok::YieldStar=>out.push(33),
            Tok::Slash=>out.push(34),
            Tok::Bang=>out.push(35),
            Tok::Question=>out.push(36),
            Tok::Pipe=>out.push(37),
            Tok::PipeGt=>out.push(38),
            Tok::PipeLt=>out.push(39),
            Tok::Amp=>out.push(40),
            Tok::Caret=>out.push(41),
            Tok::Tilde=>out.push(42),
            Tok::At=>out.push(43),
            Tok::Eof=>out.push(44),
        }
        if out.len()>AGGREGATE_CAP{return Err(err("E_SOURCE_LIMIT","registry bound token witness budget exceeded"));}
    }
    Ok(hex_digest(&out))
}
pub(crate) fn prepare(root:Snapshot<'_>,deps:&[RegistryDependency<'_>],target:TargetOs,
    compiler:&str,toolchain:&str,previous:&RegistryMemo)->Result<Prepared,Diagnostic>{
    if root.sources.len()!=1||deps.is_empty()||deps.len()>16{
        return Err(err("E_UNSUPPORTED","registry cache requires one application source and 1-16 approved dependencies"));
    }
    let mut total=0usize;
    for sources in std::iter::once(root.sources).chain(deps.iter().map(|d|d.sources)){
        if sources.len()>64{return Err(err("E_SOURCE_LIMIT","registry inventory budget exceeded"));}
        for s in sources{
            total=total.checked_add(s.source.len()).ok_or_else(||err("E_SOURCE_LIMIT","registry aggregate budget exceeded"))?;
            if total>AGGREGATE_CAP{return Err(err("E_SOURCE_LIMIT","registry aggregate budget exceeded"));}
        }
    }
    library::with_target_os(target,||{
        let root_sources=package_resolver::verify_registry_snapshot(root.manifest,root.lockfile,root.trusted_lock_sha256,root.sources)?;
        let metadata=package_resolver::parse_package_manifest(root.manifest)?;
        let identity=metadata.identity();
        let source=root_sources.get(root.entry).ok_or_else(||err("E_LOCK_HASH","root entry is not pinned"))?;
        let mut ordered=BTreeMap::new();
        for d in deps{
            if d.name==metadata.name||ordered.insert(d.name,d).is_some(){
                return Err(err("E_AMBIGUOUS","duplicate or self registry dependency"));
            }
        }
        let mut memo=RegistryMemo{target:Some(target),providers:BTreeMap::new()};
        let mut work=Work::default();
        for d in ordered.values(){
            let cached=if previous.target==Some(target){
                previous.providers.get(d.name).filter(|p|p.pin==d.trusted_lock_sha256)
            }else{None};
            let view=if let Some(cached)=cached{
                // A matching expected pin is not enough: verify supplied
                // current manifest, lock, inventory and raw source bytes.
                package_resolver::verify_registry_snapshot(d.manifest,d.lockfile,d.trusted_lock_sha256,d.sources)?;
                work.reused+=1;
                Arc::clone(&cached.view)
            }else{
                work.rebuilt+=1;
                Arc::new(FrozenRegistry::verify(d.name,d.manifest,d.lockfile,d.trusted_lock_sha256,d.sources)?)
            };
            memo.providers.insert(d.name.to_owned(),MemoProvider{pin:d.trusted_lock_sha256.to_owned(),view});
        }
        let refs:Vec<_>=memo.providers.values().map(|p|p.view.as_ref()).collect();
        let tokens=registry_namespace::prepare_registry_guest(source,&refs)?;
        let bindings=token_digest(&tokens)?;
        // All approved dependency locks are included, even if unused.
        // This conservative profile never reuses against a changed provider.
        let dependencies:Vec<_>=ordered.values().map(|d|key::Dependency{identity:d.name,digest:d.trusted_lock_sha256}).collect();
        let target_hash=hex_digest(format!("atto-wasm-stage-v1|wasm32|abi={}|os={}",crate::ABI_VERSION,
            match target{TargetOs::Posix=>"posix",TargetOs::Windows=>"windows"}).as_bytes());
        let config=hex_digest(b"registry-root-v1|single-file|explicit-host-approved-dependency-set|ordinary-registry-compiler");
        let encoded=key::encode(&key::Input{stage:"registry-wasm-module",owner:&identity,unit_kind:"source-unit",unit:root.entry,
            compiler,toolchain,target:&target_hash,configuration:&config,sources:root.trusted_lock_sha256,bindings:&bindings,
            generated:None,dependencies:&dependencies}).map_err(|e|err("E_CACHE_KEY",e))?;
        let mut bytes=[0u8;32];for(i,b)in bytes.iter_mut().enumerate(){*b=u8::from_str_radix(&encoded[i*2..i*2+2],16).map_err(|_|err("E_CACHE_KEY","invalid encoded key"))?;}
        Ok(Prepared{key:bytes,tokens,target,memo,work})
    })
}
