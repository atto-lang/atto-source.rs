//! Draft ATTOST02 bounded owned-state codec. No automatic disk or actor admission.
use crate::ast::{ActorDecl, ActorPersistenceMode, Type};
use std::collections::BTreeSet;

pub const MAX_FIELD_BYTES: usize = 16_384;
pub const MAX_SNAPSHOT_BYTES: usize = 65_536;
pub const MAX_FIELDS: usize = 256;
const ERROR: &str = "E_DURABLE_BOUNDED_CHECKPOINT_V2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind { Int = 1, Bool = 2, String = 3, Bytes = 4 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field { pub name: String, pub kind: Kind }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value { Int(i64), Bool(bool), String(String), Bytes(Vec<u8>) }

/// Private immutable metadata prevents schema/tag drift after validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schema { fingerprint: u64, fields: Vec<Field> }

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}
fn valid_name(n: &str) -> bool {
    let alpha = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    n.len() <= 128 && n.bytes().next().is_some_and(alpha)
        && n.bytes().all(|b| alpha(b) || b.is_ascii_digit())
}
fn sized(sig: &mut Vec<u8>, name: &str) {
    sig.extend_from_slice(&(name.len() as u32).to_le_bytes());
    sig.extend_from_slice(name.as_bytes());
}
fn room(out: &[u8], more: usize) -> Result<(), &'static str> {
    if more > (MAX_SNAPSHOT_BYTES - 8).saturating_sub(out.len()) { Err(ERROR) } else { Ok(()) }
}
fn take<'a>(bytes: &'a [u8], p: &mut usize, n: usize) -> Result<&'a [u8], &'static str> {
    let end = p.checked_add(n).ok_or(ERROR)?;
    let v = bytes.get(*p..end).ok_or(ERROR)?;
    *p = end;
    Ok(v)
}
fn fixed<const N: usize>(bytes: &[u8], p: &mut usize) -> Result<[u8; N], &'static str> {
    take(bytes, p, N)?.try_into().map_err(|_| ERROR)
}

impl Schema {
    /// Trusted compiler-owned schema manifest. Bytes is manifest-only until the
    /// frontend has a proven owned byte array type; arbitrary arrays are denied.
    pub fn new(actor: &str, fields: Vec<Field>) -> Result<Self, &'static str> {
        if !valid_name(actor) || fields.len() > MAX_FIELDS { return Err(ERROR); }
        let mut names = BTreeSet::new();
        let mut sig = b"atto.actor.state.schema.v2\0".to_vec();
        sized(&mut sig, actor);
        for f in &fields {
            if !valid_name(&f.name) || !names.insert(f.name.as_str()) { return Err(ERROR); }
            sized(&mut sig, &f.name);
            sig.push(f.kind as u8);
        }
        Ok(Self { fingerprint: hash(&sig), fields })
    }
    /// Derive metadata from the actual parser AST, without enabling runtime emission.
    pub fn for_actor(actor: &ActorDecl) -> Result<Self, &'static str> {
        if actor.persistence != ActorPersistenceMode::Durable || actor.fields.len() > MAX_FIELDS {
            return Err(ERROR);
        }
        let mut fields = Vec::with_capacity(actor.fields.len());
        for f in &actor.fields {
            let kind = match f.ty {
                Type::Int => Kind::Int,
                Type::Bool => Kind::Bool,
                Type::Str => Kind::String,
                _ => return Err(ERROR),
            };
            fields.push(Field { name: f.name.clone(), kind });
        }
        Self::new(&actor.name, fields)
    }
    pub fn fingerprint(&self) -> u64 { self.fingerprint }
    pub fn encode(&self, values: &[Value]) -> Result<Vec<u8>, &'static str> {
        if values.len() != self.fields.len() { return Err(ERROR); }
        let mut out = b"ATTOST02\x02".to_vec();
        out.extend_from_slice(&self.fingerprint.to_le_bytes());
        out.extend_from_slice(&(self.fields.len() as u16).to_le_bytes());
        for (f, v) in self.fields.iter().zip(values) {
            room(&out, 1)?;
            out.push(f.kind as u8);
            match (f.kind, v) {
                (Kind::Int, Value::Int(n)) => { room(&out, 8)?; out.extend_from_slice(&n.to_le_bytes()); }
                (Kind::Bool, Value::Bool(b)) => { room(&out, 1)?; out.push(u8::from(*b)); }
                (Kind::String, Value::String(s)) => Self::append_variable(&mut out, s.as_bytes())?,
                (Kind::Bytes, Value::Bytes(b)) => Self::append_variable(&mut out, b)?,
                _ => return Err(ERROR),
            }
        }
        let checksum = hash(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        Ok(out)
    }
    fn append_variable(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), &'static str> {
        if bytes.len() > MAX_FIELD_BYTES { return Err(ERROR); }
        room(out, 4 + bytes.len())?;
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
        Ok(())
    }
    pub fn decode(&self, bytes: &[u8]) -> Result<Vec<Value>, &'static str> {
        if !(27..=MAX_SNAPSHOT_BYTES).contains(&bytes.len()) { return Err(ERROR); }
        let (body, tail) = bytes.split_at(bytes.len() - 8);
        if hash(body).to_le_bytes().as_slice() != tail { return Err(ERROR); }
        let mut p = 0;
        if take(body, &mut p, 9)? != b"ATTOST02\x02"
            || u64::from_le_bytes(fixed(body, &mut p)?) != self.fingerprint
            || usize::from(u16::from_le_bytes(fixed(body, &mut p)?)) != self.fields.len() {
            return Err(ERROR);
        }
        let mut out = Vec::with_capacity(self.fields.len());
        for f in &self.fields {
            if take(body, &mut p, 1)?[0] != f.kind as u8 { return Err(ERROR); }
            let v = match f.kind {
                Kind::Int => Value::Int(i64::from_le_bytes(fixed(body, &mut p)?)),
                Kind::Bool => match take(body, &mut p, 1)?[0] {
                    0 => Value::Bool(false), 1 => Value::Bool(true), _ => return Err(ERROR),
                },
                Kind::String | Kind::Bytes => {
                    let n = u32::from_le_bytes(fixed(body, &mut p)?) as usize;
                    if n > MAX_FIELD_BYTES { return Err(ERROR); }
                    let raw = take(body, &mut p, n)?;
                    if f.kind == Kind::String {
                        Value::String(std::str::from_utf8(raw).map_err(|_| ERROR)?.to_owned())
                    } else { Value::Bytes(raw.to_vec()) }
                }
            };
            out.push(v);
        }
        if p != body.len() { return Err(ERROR); }
        Ok(out)
    }
}
