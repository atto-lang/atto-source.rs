//! Checked read-only structural *projection witness*, not a guest calling convention.
//!
//! A structurally compatible class is not layout-compatible with the nominal
//! class used to spell a formal. The lowerer must use the actual origin's
//! offsets/key witness and enforce the owner loan. No pointer casts, getters,
//! writes, or runtime map key guesses are authorized by this module.
//! The production structural-formal gate stays in place until these plans
//! have corresponding checked typed IR and Wasm emission.

use crate::diag::{Diagnostic, Pos};
use std::collections::BTreeSet;

const MAX_FIELDS: usize = 32;
const MAX_UNION: usize = 16;
const MAX_TYPE_DEPTH: usize = 8;

type R<T> = Result<T, Diagnostic>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scalar {
    Int,
    Bool,
    Float,
    Str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Literal {
    Int(i64),
    Bool(bool),
    Str(String),
}

impl Literal {
    fn scalar(&self) -> Scalar {
        match self {
            Self::Int(_) => Scalar::Int,
            Self::Bool(_) => Scalar::Bool,
            Self::Str(_) => Scalar::Str,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadType {
    Scalar(Scalar),
    /// Guaranteed singleton type, NOT a default/observed instance value.
    Singleton(Literal),
    Union(Vec<ReadType>),
}

fn bounded_type(t: &ReadType, depth: usize, pos: Pos) -> R<()> {
    if depth > MAX_TYPE_DEPTH {
        return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "structural type nesting exceeds limit"));
    }
    if let ReadType::Union(variants) = t {
        if variants.is_empty() || variants.len() > MAX_UNION {
            return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "structural union width must be 1..=16"));
        }
        for v in variants {
            bounded_type(v, depth + 1, pos)?;
        }
    }
    Ok(())
}

/// Read covariance, including the universal rule for a source union:
/// EACH possible source alternative must fit the expected type.
fn read_subtype(actual: &ReadType, expected: &ReadType) -> bool {
    match (actual, expected) {
        (ReadType::Union(items), _) => {
            !items.is_empty() && items.iter().all(|item| read_subtype(item, expected))
        }
        (_, ReadType::Union(items)) => {
            items.iter().any(|item| read_subtype(actual, item))
        }
        (ReadType::Scalar(a), ReadType::Scalar(b)) => a == b,
        (ReadType::Singleton(a), ReadType::Singleton(b)) => a == b,
        (ReadType::Singleton(a), ReadType::Scalar(b)) => a.scalar() == *b,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    Class,
    Record,
    Struct,
    Map,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadPath {
    /// Byte offset in the ACTUAL class/record/struct, not the formal's layout.
    Offset(u32),
    /// Static map key guarantee, tied to a compiler-owned proof (nonzero).
    GuaranteedKey { key: String, proof: u64 },
    /// One immutable lookup snapshot used for presence AND value (nonzero).
    SnapshotKey { key: String, snapshot: u64 },
    /// No proof of key presence. Generic Map<K,V> is not sufficient.
    UnprovenKey(String),
    /// Could invoke effects, synthesize values or bypass privacy.
    Getter,
}

#[derive(Clone, Debug)]
pub struct SourceField {
    pub name: String,
    pub ty: ReadType,
    pub public: bool,
    pub protected_state: bool,
    /// Presence must be proven even for a wildcard field.
    pub guaranteed_present: bool,
    pub path: ReadPath,
}

#[derive(Clone, Debug)]
pub struct RequestedField {
    pub name: String,
    pub ty: ReadType,
}

/// The caller/checker owns proof of the active lexical loan. A witness
/// cannot be compiled without a live owner and an exclusive-writer fence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoanEvidence {
    pub owner_slot: u32,
    pub live_for_call: bool,
    pub stable_against_writes: bool,
}

pub struct SourceShape {
    pub origin: Origin,
    pub fields: Vec<SourceField>,
    pub loan: LoanEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedField {
    pub name: String,
    pub read_ty: ReadType,
    pub path: ReadPath,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadonlyProjection {
    pub origin: Origin,
    pub owner_slot: u32,
    pub fields: Vec<ProjectedField>,
}

/// Produces a field-by-field proof, never a nominal cast. The descriptor
/// itself gives no ability to write or let the borrowed receiver escape.
pub fn plan_readonly(source: &SourceShape, expected: &[RequestedField], pos: Pos) -> R<ReadonlyProjection> {
    if !source.loan.live_for_call || !source.loan.stable_against_writes {
        return Err(Diagnostic::new(pos, "E_BORROW",
            "structural projection requires a live owner and a write-stable readonly loan"));
    }
    if expected.is_empty() || expected.len() > MAX_FIELDS || source.fields.len() > MAX_FIELDS {
        return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT",
            "structural projection field width must be 1..=32"));
    }
    let mut source_names = BTreeSet::new();
    let mut offsets = BTreeSet::new();
    for field in &source.fields {
        if !source_names.insert(field.name.as_str()) {
            return Err(Diagnostic::new(pos, "E_DUPLICATE", "duplicate source field descriptor"));
        }
        bounded_type(&field.ty, 0, pos)?;
        match (&source.origin, &field.path) {
            (Origin::Map, ReadPath::GuaranteedKey { key, proof }) if
                key == &field.name && *proof != 0 => (),
            (Origin::Map, ReadPath::SnapshotKey { key, snapshot }) if
                key == &field.name && *snapshot != 0 => (),
            (Origin::Map, ReadPath::UnprovenKey(_)) => (),
            (Origin::Map, _) => return Err(Diagnostic::new(pos, "E_TYPE",
                "map fields require a matching nonzero key-presence or snapshot witness")),
            (_, ReadPath::Offset(byte_offset)) => {
                if !offsets.insert(*byte_offset) {
                    return Err(Diagnostic::new(pos, "E_TYPE",
                        "ambiguous source field layout offsets"));
                }
            }
            (_, ReadPath::Getter) => (),
            _ => return Err(Diagnostic::new(pos, "E_TYPE",
                "static aggregate fields require proven concrete layout offsets")),
        }
    }
    let mut requested = BTreeSet::new();
    let mut fields = Vec::with_capacity(expected.len());
    for req in expected {
        if !requested.insert(req.name.as_str()) {
            return Err(Diagnostic::new(pos, "E_DUPLICATE", "duplicate required structural field"));
        }
        bounded_type(&req.ty, 0, pos)?;
        let actual = source.fields.iter().find(|f| f.name == req.name)
            .ok_or_else(|| Diagnostic::new(pos, "E_TYPE",
                format!("missing required structural field '{}'", req.name)))?;
        if !actual.public {
            return Err(Diagnostic::new(pos, "E_PRIVATE",
                format!("structural field '{}' is not public", req.name)));
        }
        if actual.protected_state {
            return Err(Diagnostic::new(pos, "E_STATE_BORROW",
                format!("sealed state field '{}' cannot be projected", req.name)));
        }
        if !actual.guaranteed_present {
            return Err(Diagnostic::new(pos, "E_TYPE",
                format!("required structural field '{}' has no presence proof", req.name)));
        }
        match &actual.path {
            ReadPath::Getter => return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "structural field projections must not invoke getters")),
            ReadPath::UnprovenKey(_) => return Err(Diagnostic::new(pos, "E_TYPE",
                "generic map key type is not a key-presence proof")),
            _ => (),
        }
        if !read_subtype(&actual.ty, &req.ty) {
            return Err(Diagnostic::new(pos, "E_TYPE",
                format!("structural field '{}' does not satisfy the required read type", req.name)));
        }
        fields.push(ProjectedField {
            name: req.name.clone(),
            read_ty: actual.ty.clone(),
            path: actual.path.clone(),
        });
    }
    Ok(ReadonlyProjection {
        origin: source.origin,
        owner_slot: source.loan.owner_slot,
        fields,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvalStep {
    /// The expression is evaluated exactly once in this source-order index.
    pub source_argument: usize,
    pub parameter: usize,
    /// Temporary write index; values are reordered only AFTER all evaluation.
    pub temp: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallEvaluationPlan {
    pub evaluations: Vec<EvalStep>,
    /// For each declared parameter, index of its source-ordered temp.
    pub parameter_temps: Vec<usize>,
}

/// Turns the syntax binder's source-ordered formal indices into an explicit
/// evaluate-first, reorder-second plan without cloning/reexecuting effects.
pub fn plan_call_evaluation(source_to_formal: &[usize], arity: usize, pos: Pos) -> R<CallEvaluationPlan> {
    if arity > MAX_FIELDS || source_to_formal.len() != arity {
        return Err(Diagnostic::new(pos, "E_ARITY", "named call arguments do not fill all parameters"));
    }
    let mut parameter_temps = vec![usize::MAX; arity];
    let mut evaluations = Vec::with_capacity(arity);
    for (src, &formal) in source_to_formal.iter().enumerate() {
        if formal >= arity {
            return Err(Diagnostic::new(pos, "E_ARITY", "argument names an out-of-range parameter"));
        }
        if parameter_temps[formal] != usize::MAX {
            return Err(Diagnostic::new(pos, "E_DUPLICATE", "named parameter is bound twice"));
        }
        parameter_temps[formal] = src;
        evaluations.push(EvalStep { source_argument: src, parameter: formal, temp: src });
    }
    if parameter_temps.contains(&usize::MAX) {
        return Err(Diagnostic::new(pos, "E_ARITY", "named call is missing a parameter"));
    }
    Ok(CallEvaluationPlan { evaluations, parameter_temps })
}
