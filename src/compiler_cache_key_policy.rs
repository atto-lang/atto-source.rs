//! In-memory candidate receipt-key lifecycle. Host provisions the secret,
//! tenant scope and monotonically advancing epoch from an independent
//! protected ledger. No disk key storage or durable anti-rollback claim.
use crate::compiler_artifact_receipt::{self as receipt,hmac,equal};
const INVALID:&str="E_CACHE_KEY_POLICY: invalid scope, epoch or key";
pub(crate) struct ActiveKey {
    scope:String,
    epoch:u64,
    master:[u8;32],
}
fn valid_scope(scope:&str)->bool{
    !scope.is_empty()&&scope.len()<=512&&scope.bytes().all(|c|(33..=126).contains(&c))
}
impl ActiveKey {
    pub(crate) fn new(scope:&str,epoch:u64,master:[u8;32])->Result<Self,&'static str>{
        if !valid_scope(scope)||epoch==0||master==[0;32]{return Err(INVALID);}
        Ok(Self{scope:scope.to_owned(),epoch,master})
    }
    pub(crate) fn rotate(&mut self,epoch:u64,master:[u8;32])->Result<(),&'static str>{
        if epoch<=self.epoch||master==[0;32]||equal(&master,&self.master){return Err(INVALID);}
        self.epoch=epoch;
        self.master=master;
        Ok(())
    }
    pub(crate) fn derived(&self)->[u8;32]{
        let mut msg=b"atto.compiler.local-receipt-key.v1\0".to_vec();
        msg.extend_from_slice(&(self.scope.len() as u32).to_be_bytes());
        msg.extend_from_slice(self.scope.as_bytes());
        msg.extend_from_slice(&self.epoch.to_be_bytes());
        hmac(&self.master,&msg)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use receipt::tests::{KEY,SECRET,PAYLOAD};
    #[test]
    fn scope_rotation_revocation_and_reconstruction(){
        let mut a=ActiveKey::new("tenant/a",1,SECRET).unwrap();
        let b=ActiveKey::new("tenant/b",1,SECRET).unwrap();
        let restarted=ActiveKey::new("tenant/a",1,SECRET).unwrap();
        assert_eq!(a.derived(),restarted.derived());
        assert_ne!(a.derived(),b.derived());
        let old=receipt::seal(&KEY,PAYLOAD,&a.derived()).unwrap();
        assert!(receipt::verify(&KEY,&old,&b.derived()).is_err());
        assert_eq!(receipt::verify(&KEY,&old,&restarted.derived()).unwrap(),PAYLOAD);
        assert!(a.rotate(1,[7;32]).is_err());
        assert!(a.rotate(2,SECRET).is_err());
        assert!(a.rotate(2,[0;32]).is_err());
        assert_eq!(receipt::verify(&KEY,&old,&a.derived()).unwrap(),PAYLOAD);
        a.rotate(2,[7;32]).unwrap();
        assert!(receipt::verify(&KEY,&old,&a.derived()).is_err());
        assert!(a.rotate(1,SECRET).is_err());
        let new=receipt::seal(&KEY,PAYLOAD,&a.derived()).unwrap();
        assert!(receipt::verify(&KEY,&new,&restarted.derived()).is_err());
        assert_eq!(receipt::verify(&KEY,&new,&ActiveKey::new("tenant/a",2,[7;32]).unwrap().derived()).unwrap(),PAYLOAD);
        assert!(ActiveKey::new("tenant\n/a",1,SECRET).is_err());
        assert!(ActiveKey::new("tenant/a",0,SECRET).is_err());
        assert!(ActiveKey::new("tenant/a",1,[0;32]).is_err());
        println!("CACHE_KEY_POLICY|tenant-isolation|epoch-rotation|old-receipt-rejected|restart");
    }
}
