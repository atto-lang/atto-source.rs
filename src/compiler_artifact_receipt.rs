//! Candidate host trusted-ledger receipt. A verified envelope is NOT an
//! executable authorization. Stage witnesses and publisher authority remain
//! separate gates; do not put the secret in the cache or expose it to guests.
use crate::sha256::sha256;
pub const HEADER: usize = 112;
pub const MAX_ARTIFACT: usize = 4 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"ATTOCAC1";
const DOMAIN: &[u8] = b"atto.compiler.artifact-receipt.v1\0";
const INVALID: &str = "E_CACHE_RECEIPT: invalid envelope";
pub(crate) fn hmac(secret: &[u8; 32], message: &[u8]) -> [u8; 32] {
    // RFC 2104 HMAC with the existing SHA-256 primitive and fixed 32-byte key.
    // Candidate implementation; independent crypto review before activation.
    let mut inner = vec![0x36; 64];
    let mut outer = vec![0x5c; 64];
    for (i, &byte) in secret.iter().enumerate() {
        inner[i] ^= byte;
        outer[i] ^= byte;
    }
    inner.extend_from_slice(message);
    outer.extend_from_slice(&sha256(&inner));
    sha256(&outer)
}
fn tag(secret: &[u8; 32], header: &[u8]) -> [u8; 32] {
    let mut message=Vec::with_capacity(DOMAIN.len()+80);
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(&header[..80]);
    hmac(secret,&message)
}
pub(crate) fn equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    // Fixed-length comparison without data-dependent early return.
    // Not a claim of independently audited machine-code constant time.
    a.iter().zip(b).fold(0u8, |acc, (&x, &y)| acc | (x ^ y)) == 0
}
pub fn seal(key: &[u8; 32], artifact: &[u8], secret: &[u8; 32]) -> Result<Vec<u8>, &'static str> {
    if artifact.len() > MAX_ARTIFACT { return Err(INVALID); }
    let mut bytes = Vec::with_capacity(HEADER + artifact.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(key);
    bytes.extend_from_slice(&(artifact.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&sha256(artifact));
    bytes.extend_from_slice(&tag(secret, &bytes));
    bytes.extend_from_slice(artifact);
    Ok(bytes)
}
pub fn verify(key: &[u8; 32], envelope: &[u8], secret: &[u8; 32]) -> Result<Vec<u8>, &'static str> {
    if envelope.len() < HEADER || envelope.len() > HEADER + MAX_ARTIFACT { return Err(INVALID); }
    if &envelope[..8] != MAGIC || !equal(key, &envelope[8..40]) ||
        u64::from_be_bytes(envelope[40..48].try_into().map_err(|_| INVALID)?) != (envelope.len()-HEADER) as u64 ||
        !equal(&tag(secret, envelope), &envelope[80..112]) ||
        !equal(&sha256(&envelope[HEADER..]), &envelope[48..80]) {
        return Err(INVALID);
    }
    Ok(envelope[HEADER..].to_vec())
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) const KEY: [u8; 32] = [0x6d,0x06,0x58,0xc4,0x1d,0x4e,0x45,0xed,0xe0,0x29,0xca,0x0c,0x76,0x54,0x33,0x3c,0x5a,0xa0,0xbc,0x11,0x7f,0x31,0x79,0xf1,0x28,0x32,0xaa,0x7b,0x1d,0x22,0xb6,0x57];
    pub(crate) const SECRET: [u8; 32] = [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31];
    pub(crate) const PAYLOAD: &[u8] = b"atto-artifact-v1\0\xff";
    #[test]
    fn vector_tampering_and_budget() {
        let bytes=seal(&KEY,PAYLOAD,&SECRET).unwrap();
        let actual=bytes[80..112].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("");
        assert_eq!(actual,"3653396c79c689b07bb08274d60b4de17d57d0c9dd4fd22180aa7af88940b524");
        assert_eq!(verify(&KEY,&bytes,&SECRET).unwrap(),PAYLOAD);
        for index in [0,8,40,48,80,112,bytes.len()-1] {
            let mut changed=bytes.clone();changed[index]^=1;
            assert!(verify(&KEY,&changed,&SECRET).is_err());
        }
        let mut forged=bytes.clone();forged[112]^=1;
        let hash=sha256(&forged[112..]);forged[48..80].copy_from_slice(&hash);
        assert!(verify(&KEY,&forged,&SECRET).is_err());
        assert!(verify(&[0;32],&bytes,&SECRET).is_err());
        assert!(verify(&KEY,&bytes,&[0;32]).is_err());
        assert!(verify(&KEY,&bytes[..111],&SECRET).is_err());
        assert!(verify(&KEY,&bytes[..bytes.len()-1],&SECRET).is_err());
        let mut appended=bytes.clone();appended.push(0);
        assert!(verify(&KEY,&appended,&SECRET).is_err());
        assert!(verify(&KEY,&vec![0;HEADER+MAX_ARTIFACT+1],&SECRET).is_err());
        assert!(seal(&KEY,&vec![0;MAX_ARTIFACT+1],&SECRET).is_err());
        assert!(verify(&KEY,&seal(&KEY,&[],&SECRET).unwrap(),&SECRET).unwrap().is_empty());
        println!("CACHE_RECEIPT_RUST|tag={actual}|forgery-rejected|bounded");
    }
}
