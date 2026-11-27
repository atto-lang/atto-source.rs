//! Canonical cache key v1 envelope. No source authentication or artifact trust.
use crate::sha256::hex_digest;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct Dependency<'a> {
    pub identity: &'a str,
    pub digest: &'a str,
}
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub stage: &'a str,
    pub owner: &'a str,
    pub unit_kind: &'a str,
    pub unit: &'a str,
    pub compiler: &'a str,
    pub toolchain: &'a str,
    pub target: &'a str,
    pub configuration: &'a str,
    pub sources: &'a str,
    pub bindings: &'a str,
    pub generated: Option<&'a str>,
    pub dependencies: &'a [Dependency<'a>],
}
fn identity(out: &mut Vec<u8>, value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 512 || !value.bytes().all(|c| (33..=126).contains(&c)) {
        return Err("E_CACHE_KEY: identity");
    }
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
fn digest(out: &mut Vec<u8>, value: &str) -> Result<(), &'static str> {
    fn nibble(b: u8) -> Result<u8, &'static str> {
        match b {
            b'0'..=b'9' => Ok(b-b'0'),
            b'a'..=b'f' => Ok(b-b'a'+10),
            _ => Err("E_CACHE_KEY: digest"),
        }
    }
    if value.len() != 64 { return Err("E_CACHE_KEY: digest"); }
    for pair in value.as_bytes().chunks_exact(2) {
        out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    Ok(())
}
pub fn encode(input: &Input<'_>) -> Result<String, &'static str> {
    if !matches!(input.unit_kind, "namespace" | "source-unit") {
        return Err("E_CACHE_KEY: unit kind");
    }
    if input.dependencies.len() > 128 { return Err("E_CACHE_KEY: dependency budget"); }
    let mut bytes=b"atto.compiler.cache-key.v1\0".to_vec();
    for field in [input.stage,input.owner,input.unit_kind,input.unit] {
        identity(&mut bytes,field)?;
    }
    for field in [input.compiler,input.toolchain,input.target,input.configuration,input.sources,input.bindings] {
        digest(&mut bytes,field)?;
    }
    match input.generated {
        None => bytes.push(0),
        Some(value) => { bytes.push(1); digest(&mut bytes,value)?; }
    }
    let mut ordered=BTreeMap::new();
    for dep in input.dependencies {
        if ordered.insert(dep.identity,dep.digest).is_some() {
            return Err("E_CACHE_KEY: duplicate dependency");
        }
    }
    bytes.extend_from_slice(&(ordered.len() as u32).to_be_bytes());
    for (id,hash) in ordered { identity(&mut bytes,id)?;digest(&mut bytes,hash)?; }
    Ok(hex_digest(&bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_drift_and_hostile_inputs() {
        let hashes: Vec<String>=(0..10).map(|n|format!("{n:02x}").repeat(32)).collect();
        let deps=[Dependency{identity:"@org/project/b",digest:&hashes[7]},Dependency{identity:"@org/project/c",digest:&hashes[8]}];
        let b=Input{stage:"typed-ir",owner:"@org/project/a",unit_kind:"namespace",unit:"network",
            compiler:&hashes[1],toolchain:&hashes[2],target:&hashes[3],configuration:&hashes[4],
            sources:&hashes[5],bindings:&hashes[6],generated:None,dependencies:&deps};
        let expected="6d0658c41d4e45ede029ca0c7654333c5aa0bc117f3179f12832aa7b1d22b657";
        assert_eq!(encode(&b).unwrap(),expected);
        assert_eq!(encode(&Input{dependencies:&[deps[1],deps[0]],..b}).unwrap(),expected);
        let changed=[Input{compiler:&hashes[9],..b},Input{toolchain:&hashes[9],..b},
            Input{target:&hashes[9],..b},Input{configuration:&hashes[9],..b},Input{sources:&hashes[9],..b},
            Input{bindings:&hashes[9],..b},Input{generated:Some(&hashes[0]),..b},Input{stage:"native-code",..b},
            Input{owner:"@org/project/z",..b},Input{unit_kind:"source-unit",..b},Input{unit:"other",..b},
            Input{dependencies:&[],..b}];
        for value in changed {assert_ne!(encode(&value).unwrap(),expected);}
        assert_ne!(encode(&Input{dependencies:&[Dependency{digest:&hashes[9],..deps[0]},deps[1]],..b}).unwrap(),expected);
        assert_ne!(encode(&Input{owner:"ab",unit:"c",..b}).unwrap(),encode(&Input{owner:"a",unit:"bc",..b}).unwrap());
        for bad in ["","a\n","a\0b","é",&"x".repeat(513)] {assert!(encode(&Input{owner:bad,..b}).is_err());}
        for bad in ["",&"F".repeat(64),&format!("{}\n",hashes[1])] {
            assert!(encode(&Input{compiler:bad,..b}).is_err());
            assert!(encode(&Input{generated:Some(bad),..b}).is_err());
        }
        assert!(encode(&Input{unit_kind:"package",..b}).is_err());
        assert!(encode(&Input{dependencies:&[deps[0],deps[0]],..b}).is_err());
        assert!(encode(&Input{dependencies:&[deps[0];129],..b}).is_err());
        println!("CACHE_KEY_RUST|vector={expected}|permutation|field-drift|framing|malformed-rejected");
    }
}
