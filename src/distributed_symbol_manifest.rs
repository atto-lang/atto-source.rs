//! Distributed Symbol v1 manifest admission and local-ordinal translation.
//! Host/compiler foundation only: no new guest ABI or transport is admitted.
use std::collections::{BTreeMap, BTreeSet};

pub type Uuid = [u8; 16];
pub type Declaration = (String, String);
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolError {
    Malformed, Duplicate, Capacity, Unknown, Denied, InvalidOrdinal,
}
pub fn valid_uuid(id: &Uuid) -> bool {
    id[6] & 0xf0 == 0x40 && id[8] & 0xc0 == 0x80
}
pub fn decode(wire: &[u8]) -> Result<Uuid, SymbolError> {
    let id: Uuid = wire.try_into().map_err(|_| SymbolError::Malformed)?;
    if !valid_uuid(&id) { return Err(SymbolError::Malformed); }
    Ok(id)
}

#[derive(Debug)]
pub struct Catalog {
    forward: BTreeMap<Declaration, Uuid>,
    reverse: BTreeMap<Uuid, Declaration>,
}
impl Catalog {
    pub fn new(bindings: &[(Declaration, Uuid)]) -> Result<Self, SymbolError> {
        if bindings.len() > MAX_ENTRIES { return Err(SymbolError::Capacity); }
        let mut forward = BTreeMap::new();
        let mut reverse = BTreeMap::new();
        let mut bytes = 0;
        for (declaration, id) in bindings {
            let (namespace, name) = declaration;
            if namespace.is_empty() || namespace.len() > 256 || name.is_empty()
                || name.len() > 256 || !valid_uuid(id) {
                return Err(SymbolError::Malformed);
            }
            let cost = namespace.len() + name.len() + 16;
            if cost > MAX_BYTES - bytes { return Err(SymbolError::Capacity); }
            bytes += cost;
            if forward.insert(declaration.clone(), *id).is_some()
                || reverse.insert(*id, declaration.clone()).is_some() {
                return Err(SymbolError::Duplicate);
            }
        }
        Ok(Self { forward, reverse })
    }
    /// Compiler-owned manifest lookup. Visibility is checked by its caller.
    pub fn resolve(&self, declaration: &Declaration) -> Result<Uuid, SymbolError> {
        self.forward.get(declaration).copied().ok_or(SymbolError::Unknown)
    }
    /// Explicit grants originate with the trusted admission layer, never guest metadata.
    pub fn receive(&self, wire: &[u8], grants: &BTreeSet<Declaration>) -> Result<Uuid, SymbolError> {
        let id = decode(wire)?;
        let declaration = self.reverse.get(&id).ok_or(SymbolError::Unknown)?;
        if !grants.contains(declaration) { return Err(SymbolError::Denied); }
        Ok(id)
    }
    /// Build the translation for one module's public Wasm ordinal table.
    /// The namespace and ordered spelling table must come from trusted compiler
    /// metadata. Private ordinals are deliberately not transportable here.
    pub fn public_ordinals(&self, namespace: &str, names: &[String])
        -> Result<PublicOrdinals, SymbolError> {
        if names.len() > MAX_ENTRIES { return Err(SymbolError::Capacity); }
        let mut ids = Vec::with_capacity(names.len());
        let mut seen = BTreeSet::new();
        for name in names {
            let name = name.strip_prefix('#').ok_or(SymbolError::Malformed)?;
            if name.is_empty() || name.len() > 256 || namespace.is_empty() || namespace.len() > 256 {
                return Err(SymbolError::Malformed);
            }
            let id = self.resolve(&(namespace.to_owned(), name.to_owned()))?;
            if !seen.insert(id) { return Err(SymbolError::Duplicate); }
            ids.push(id);
        }
        Ok(PublicOrdinals(ids))
    }
}
#[derive(Debug)]
pub struct PublicOrdinals(Vec<Uuid>);
impl PublicOrdinals {
    pub fn uuid(&self, ordinal: i64) -> Result<Uuid, SymbolError> {
        let index = ordinal.checked_sub(1).and_then(|n| usize::try_from(n).ok())
            .ok_or(SymbolError::InvalidOrdinal)?;
        self.0.get(index).copied().ok_or(SymbolError::InvalidOrdinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: Uuid = [0x00,0x11,0x22,0x33,0x44,0x55,0x46,0x77,0x88,0x99,0xaa,0xbb,0xcc,0xdd,0xee,0xff];
    const B: Uuid = [0xff,0xee,0xdd,0xcc,0xbb,0xaa,0x49,0x88,0xb7,0x66,0x55,0x44,0x33,0x22,0x11,0x00];
    fn key(ns: &str, name: &str) -> Declaration { (ns.into(), name.into()) }
    #[test]
    fn independent_catalogs_and_ordinal_spaces() {
        let a = key("fixture/core", "ready");
        let b = key("fixture/app", "ready");
        let first = Catalog::new(&[(a.clone(), A), (b.clone(), B)]).unwrap();
        let second = Catalog::new(&[(b.clone(), B), (a.clone(), A)]).unwrap();
        let p = first.public_ordinals("fixture/core", &["#ready".into()]).unwrap();
        let q = second.public_ordinals("fixture/app", &["#ready".into()]).unwrap();
        assert_eq!(p.uuid(1), Ok(A));
        assert_eq!(q.uuid(1), Ok(B));
        assert_eq!(second.receive(&A, &BTreeSet::from([a])), Ok(A));
        assert_eq!(second.receive(&A, &BTreeSet::from([b])), Err(SymbolError::Denied));
        for n in [i64::MIN, -1, 0, 2, i64::MAX] { assert_eq!(p.uuid(n), Err(SymbolError::InvalidOrdinal)); }
    }
    #[test]
    fn duplicate_manifest_and_missing_names_fail_closed() {
        let a = key("fixture/core", "ready");
        let b = key("fixture/app", "ready");
        assert_eq!(Catalog::new(&[(a.clone(), A),(b,A)]).unwrap_err(), SymbolError::Duplicate);
        assert_eq!(Catalog::new(&[(a.clone(),A),(a.clone(),B)]).unwrap_err(), SymbolError::Duplicate);
        let c = Catalog::new(&[(a,A)]).unwrap();
        assert_eq!(c.public_ordinals("fixture/core", &["#unknown".into()]).unwrap_err(), SymbolError::Unknown);
        assert_eq!(c.public_ordinals("fixture/core", &["#ready".into(),"#ready".into()]).unwrap_err(), SymbolError::Duplicate);
        assert_eq!(c.public_ordinals("fixture/core", &["ready".into()]).unwrap_err(), SymbolError::Malformed);
    }
    #[test]
    fn canonical_wire_and_bounds() {
        assert_eq!(decode(&A), Ok(A));
        assert_eq!(decode(&A[..15]), Err(SymbolError::Malformed));
        assert_eq!(decode(&[0;17]), Err(SymbolError::Malformed));
        let mut bad = A; bad[6] = 0x50;
        assert_eq!(decode(&bad), Err(SymbolError::Malformed));
        bad = A; bad[8] = 0x40;
        assert_eq!(decode(&bad), Err(SymbolError::Malformed));
        let c = Catalog::new(&[(key("fixture/core","ready"),A)]).unwrap();
        assert_eq!(c.receive(&B,&BTreeSet::new()), Err(SymbolError::Unknown));
        assert_eq!(Catalog::new(&vec![(key("a","b"),A);MAX_ENTRIES+1]).unwrap_err(), SymbolError::Capacity);
    }
    #[test]
    fn ordinals_are_not_global_and_reordering_does_not_change_identity() {
        let c = Catalog::new(&[(key("fixture/core","ready"),A),(key("fixture/core","stop"),B)]).unwrap();
        let p = c.public_ordinals("fixture/core", &["#ready".into(),"#stop".into()]).unwrap();
        let q = c.public_ordinals("fixture/core", &["#stop".into(),"#ready".into()]).unwrap();
        assert_eq!(p.uuid(1), q.uuid(2));
        assert_eq!(p.uuid(2), q.uuid(1));
        assert_ne!(p.uuid(1), q.uuid(1));
    }
}
