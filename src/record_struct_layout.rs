//! Static Record/Struct *logical slot* planning (host compiler only).
//!
//! This module does NOT emit a guest object, assign byte offsets, claim an
//! actual machine ABI, or enable collection values in executable Wasm.
//! A known closed Record and a nominal Struct both support constant field
//! indices without runtime hashing. An open Record needs a shape/presence
//! check; a Map uses its own dynamic-key lookup contract.
//!
//! Record type identity is independent of source field order, while every
//! source value still evaluates once in original source order. Struct keeps
//! declaration order and distinct module-qualified nominal identity.

use std::collections::BTreeMap;

use crate::collection_types::LiteralType;

pub const MAX_STATIC_SHAPES: usize = 4096;
pub const MAX_STATIC_FIELDS: usize = 1024;
pub const MAX_LAYOUT_NAME_BYTES: usize = 65_536;
const MAX_TYPE_NODES: usize = 32_768;

pub type FieldType = (String, LiteralType);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShapeKind {
    Record,
    Struct { qualified_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub kind: ShapeKind,
    /// Record: UTF-8 bytewise name sorted. Struct: declaration order.
    pub fields: Vec<FieldType>,
    /// Cached at compile time. No runtime Map is required for this lookup.
    pub field_to_slot: BTreeMap<String, usize>,
}

impl Shape {
    pub fn slot(&self, name: &str) -> Option<usize> {
        self.field_to_slot.get(name).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternedLayout {
    /// Compiler-local handle. Not a serialized runtime ABI shape ID.
    pub shape_id: usize,
    /// For each expression in its *source* order, destination static slot.
    pub source_to_slot: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    TooManyFields,
    TooManyShapes,
    TooManyNameBytes,
    DuplicateField,
    InvalidStructName,
    InvalidStructField,
    ChangedNominalDefinition,
    UnresolvedType,
    TypeBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldDecision {
    /// A compile-time known logical field index, not a byte offset.
    StaticSlot(usize),
    /// Open record values require a shape/presence/type proof before access.
    CheckOpenShape,
    /// Maps retain dynamic key-based lookup, not record field projection.
    RuntimeMapKey,
    /// The static closed record does not contain this field.
    Missing,
    NotFieldContainer,
}

pub fn decide_access(ty: &LiteralType, label: &str) -> FieldDecision {
    match ty {
        LiteralType::Record(fields) => fields
            .iter()
            .position(|(field, _)| field == label)
            .map(FieldDecision::StaticSlot)
            .unwrap_or(FieldDecision::Missing),
        LiteralType::OpenRecord => FieldDecision::CheckOpenShape,
        LiteralType::Map(_, _) => FieldDecision::RuntimeMapKey,
        _ => FieldDecision::NotFieldContainer,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ShapeKey {
    kind: ShapeKind,
    fields: Vec<FieldType>,
}

/// Owns compiler-only shape descriptors, interned by structural or nominal
/// identity. No guest heap data or mutable field values are shared.
#[derive(Debug)]
pub struct LayoutInterner {
    shapes: Vec<Shape>,
    keys: BTreeMap<ShapeKey, usize>,
    nominal_structs: BTreeMap<String, Vec<FieldType>>,
    shape_limit: usize,
}

impl Default for LayoutInterner {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutInterner {
    pub fn new() -> Self {
        Self {
            shapes: Vec::new(),
            keys: BTreeMap::new(),
            nominal_structs: BTreeMap::new(),
            shape_limit: MAX_STATIC_SHAPES,
        }
    }

    /// An optional *smaller* cap is useful for bounded compiler profiles.
    pub fn with_shape_limit(limit: usize) -> Self {
        let mut me = Self::new();
        me.shape_limit = limit.min(MAX_STATIC_SHAPES);
        me
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
    pub fn shape(&self, id: usize) -> Option<&Shape> {
        self.shapes.get(id)
    }

    /// Intern a closed Record. Return a source-evaluation-order -> canonical
    /// layout-slot permutation; no initializer expression may be reordered.
    pub fn closed_record(&mut self, source_fields: &[FieldType]) -> Result<InternedLayout, Error> {
        validate_fields(source_fields, false)?;
        let mut canonical = source_fields.to_vec();
        canonical.sort_by(|(a, _), (b, _)| a.cmp(b));
        if canonical.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(Error::DuplicateField);
        }
        let mut slots = BTreeMap::new();
        for (index, (name, _)) in canonical.iter().enumerate() {
            slots.insert(name.clone(), index);
        }
        let source_to_slot = source_fields
            .iter()
            .map(|(name, _)| slots[name.as_str()])
            .collect();
        let shape_id = self.intern(ShapeKind::Record, canonical)?;
        Ok(InternedLayout {
            shape_id,
            source_to_slot,
        })
    }

    /// Abstract static Struct layout planning. This does NOT parse, construct
    /// or lower a user Struct. Field order is fixed at declaration order,
    /// and distinct qualified names are always nominally distinct.
    pub fn nominal_struct(
        &mut self,
        qualified_name: &str,
        declared: &[FieldType],
    ) -> Result<InternedLayout, Error> {
        if qualified_name.is_empty() || qualified_name.len() > 1024 {
            return Err(Error::InvalidStructName);
        }
        validate_fields(declared, true)?;
        if let Some(existing) = self.nominal_structs.get(qualified_name) {
            if existing != declared {
                return Err(Error::ChangedNominalDefinition);
            }
        } else if self.shapes.len() >= self.shape_limit {
            return Err(Error::TooManyShapes);
        }
        let kind = ShapeKind::Struct {
            qualified_name: qualified_name.to_owned(),
        };
        let shape_id = self.intern(kind, declared.to_vec())?;
        self.nominal_structs
            .entry(qualified_name.to_owned())
            .or_insert_with(|| declared.to_vec());
        Ok(InternedLayout {
            shape_id,
            source_to_slot: (0..declared.len()).collect(),
        })
    }

    fn intern(&mut self, kind: ShapeKind, fields: Vec<FieldType>) -> Result<usize, Error> {
        let key = ShapeKey {
            kind: kind.clone(),
            fields: fields.clone(),
        };
        if let Some(&id) = self.keys.get(&key) {
            return Ok(id);
        }
        if self.shapes.len() >= self.shape_limit {
            return Err(Error::TooManyShapes);
        }
        let id = self.shapes.len();
        let field_to_slot = fields
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.clone(), i))
            .collect();
        self.shapes.push(Shape {
            kind,
            fields,
            field_to_slot,
        });
        self.keys.insert(key, id);
        Ok(id)
    }
}

fn valid_identifier(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn validate_fields(fields: &[FieldType], is_struct: bool) -> Result<(), Error> {
    if fields.len() > MAX_STATIC_FIELDS {
        return Err(Error::TooManyFields);
    }
    let mut name_bytes = 0usize;
    let mut seen = std::collections::BTreeSet::new();
    for (name, ty) in fields {
        if is_struct && !valid_identifier(name) {
            return Err(Error::InvalidStructField);
        }
        name_bytes = name_bytes
            .checked_add(name.len())
            .ok_or(Error::TooManyNameBytes)?;
        if name_bytes > MAX_LAYOUT_NAME_BYTES {
            return Err(Error::TooManyNameBytes);
        }
        if !seen.insert(name) {
            return Err(Error::DuplicateField);
        }
        validate_resolved_type(ty)?;
    }
    Ok(())
}

/// Bounded, iterative walk: a fabricated caller cannot induce deep recursive
/// stack growth or claim static layout for unresolved dynamic element types.
fn validate_resolved_type(root: &LiteralType) -> Result<(), Error> {
    let mut pending = vec![root];
    let mut remaining = MAX_TYPE_NODES;
    while let Some(next) = pending.pop() {
        if remaining == 0 {
            return Err(Error::TypeBudget);
        }
        remaining -= 1;
        match next {
            LiteralType::Bottom | LiteralType::OpenRecord | LiteralType::AnyTuple => {
                return Err(Error::UnresolvedType)
            }
            LiteralType::Record(fields) => {
                if fields.len() > MAX_STATIC_FIELDS {
                    return Err(Error::TooManyFields);
                }
                for (_, child) in fields {
                    pending.push(child);
                }
            }
            LiteralType::Array(t) | LiteralType::List(t) => pending.push(t),
            LiteralType::Map(k, v) => {
                pending.push(k);
                pending.push(v);
            }
            LiteralType::Tuple(items) => pending.extend(items),
            LiteralType::Int | LiteralType::Bool | LiteralType::Float | LiteralType::String => {}
        }
    }
    Ok(())
}
