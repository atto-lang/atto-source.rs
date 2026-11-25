//! Test-only host trust boundary: independently pinned active ledger record.
//! The expected record digest MUST come from the current protected host
//! authority on each admission, not the cache or the record being checked.
//! This is not a rollback-resistant persistence provider. Never persist master
//! keys in cache entries; the host provides a separate 32-byte master.
use crate::{compiler_cache_key_policy::ActiveKey,sha256::sha256,compiler_artifact_receipt::equal};
const MAGIC:&[u8;8]=b"ATTOLED1";
const INVALID:&str="E_CACHE_LEDGER: invalid or stale host ledger record";
const MAX:usize=8+4+512+8+32;
pub(crate) fn record(scope:&str,epoch:u64,master:[u8;32])->Result<Vec<u8>,&'static str>{
    let active=ActiveKey::new(scope,epoch,master).map_err(|_|INVALID)?;
    let mut out=Vec::with_capacity(8+4+scope.len()+8+32);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(scope.len() as u32).to_be_bytes());
    out.extend_from_slice(scope.as_bytes());
    out.extend_from_slice(&epoch.to_be_bytes());
    // Fingerprint of the domain-separated active key, not the master.
    out.extend_from_slice(&sha256(&active.derived()));
    Ok(out)
}
pub(crate) fn admit(scope:&str,bytes:&[u8],expected_digest:&[u8;32],
    master:[u8;32])->Result<ActiveKey,&'static str>{
    if bytes.len()<53||bytes.len()>MAX||!equal(&sha256(bytes),expected_digest)||&bytes[..8]!=MAGIC{
        return Err(INVALID);
    }
    let n=u32::from_be_bytes(bytes[8..12].try_into().map_err(|_|INVALID)?) as usize;
    if n==0||n>512||bytes.len()!=52+n{return Err(INVALID);}
    let actual=std::str::from_utf8(&bytes[12..12+n]).map_err(|_|INVALID)?;
    if actual!=scope{return Err(INVALID);}
    let epoch=u64::from_be_bytes(bytes[12+n..20+n].try_into().map_err(|_|INVALID)?);
    let active=ActiveKey::new(actual,epoch,master).map_err(|_|INVALID)?;
    if !equal(&sha256(&active.derived()),&bytes[20+n..]){return Err(INVALID);}
    Ok(active)
}
#[cfg(test)]
mod tests{
    use super::*;
    use crate::compiler_artifact_receipt::{self as receipt,tests::{KEY,SECRET,PAYLOAD}};
    #[test]
    fn pinned_current_record_rejects_rollback_substitution_and_malformed(){
        let old=record("tenant/a",1,SECRET).unwrap();
        let current=record("tenant/a",2,[7;32]).unwrap();
        let pin=sha256(&current);
        let a=admit("tenant/a",&current,&pin,[7;32]).unwrap();
        let stale=admit("tenant/a",&old,&sha256(&old),SECRET).unwrap();
        let artifact=receipt::seal(&KEY,PAYLOAD,&stale.derived()).unwrap();
        assert!(receipt::verify(&KEY,&artifact,&a.derived()).is_err());
        assert!(admit("tenant/a",&old,&pin,SECRET).is_err());
        assert!(admit("tenant/b",&current,&pin,[7;32]).is_err());
        assert!(admit("tenant/a",&current,&pin,SECRET).is_err());
        for i in [0,8,12,20,current.len()-1]{
            let mut tampered=current.clone();tampered[i]^=1;
            assert!(admit("tenant/a",&tampered,&pin,[7;32]).is_err());
        }
        let mut bad=current.clone();bad[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(admit("tenant/a",&bad,&sha256(&bad),[7;32]).is_err());
        for bad in [&current[..51],&current[..current.len()-1],&[0;MAX+1][..]]{
            assert!(admit("tenant/a",bad,&sha256(bad),[7;32]).is_err());
        }
        let mut extra=current.clone();extra.push(0);
        assert!(admit("tenant/a",&extra,&sha256(&extra),[7;32]).is_err());
        println!("CACHE_LEDGER|external-current-pin|rollback-rejected|wrong-tenant-key-rejected|bounded");
    }
}
