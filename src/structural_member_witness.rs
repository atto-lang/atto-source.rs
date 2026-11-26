//! Checked, non-emitting structural member witnesses for AttoLang.
//!
//! `~T` and `structural T` share this semantic contract. The caller MUST
//! derive the candidate from checked compiler declarations, never guest data.
//! Proofs retain the *actual* source type, layout epoch, and active loan.
//! There is deliberately no code emitter, foreign pointer cast, dynamic
//! reflection, or authorization to invoke a method in this module.
//!
//! The shape/field half reuses `shape_witness`. Method proofs keep the actual
//! dispatch slot and reject ambiguous, private, effectful or stronger-capability
//! implementations. A future guest ABI must separately verify slot validity,
//! loan liveness and the concrete receiver layout at the call boundary.

use std::collections::BTreeSet;
use crate::shape_witness::{
    prove_readonly_projection, readonly_subtype, ReadFieldWitness, RequiredField,
    ShapeError, ShapeTy, SourceKind, SourceShape, MAX_SHAPE_FIELDS,
};

pub const MAX_STRUCTURAL_METHODS: usize = 32;
pub const MAX_METHOD_PARAMETERS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequiredKind {
    Interface,
    Trait,
    Class,
    Struct,
    Record,
    Tuple,
    Map,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverMode {
    Readonly,
    Mutable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Passing {
    Borrow,
    BorrowMut,
    Take,
    Share,
    Copy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodContract {
    pub name: String,
    pub params: Vec<ShapeTy>,
    pub result: ShapeTy,
    pub receiver: ReceiverMode,
    pub parameter_passing: Vec<Passing>,
    pub result_passing: Passing,
    pub is_async: bool,
    /// Canonical effect and permission names; checked compiler metadata only.
    pub effects: Vec<String>,
    pub permissions: Vec<String>,
    pub generic_bounds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMethod {
    pub signature: MethodContract,
    pub public: bool,
    pub is_instance: bool,
    pub sealed_actor_state: bool,
    /// Compiler-owned slot for the ACTUAL concrete receiver type.
    pub dispatch_slot: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct RequiredMembers {
    pub kind: RequiredKind,
    /// Trusted requesting TenantVM context, never supplied by guest code.
    pub tenant_vm_id: u64,
    /// Exported instance fields, NOT private/protected/class statics.
    pub fields: Vec<RequiredField>,
    pub methods: Vec<MethodContract>,
    /// Exact positional tuple type when kind == Tuple.
    pub tuple_elements: Vec<ShapeTy>,
    /// Exact key/value types when kind == Map.
    pub map_elements: Option<(ShapeTy, ShapeTy)>,
}

#[derive(Clone, Debug)]
pub struct SourceMembers {
    /// Trusted concrete source TenantVM context, never supplied by guest code.
    pub tenant_vm_id: u64,
    /// An ordinary concrete class, struct, Record, Tuple or Map.
    pub shape: SourceShape,
    pub methods: Vec<SourceMethod>,
    pub tuple_elements: Vec<ShapeTy>,
    pub map_elements: Option<(ShapeTy, ShapeTy)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodWitness {
    pub name: String,
    /// Keep the overloaded required callable signature; name alone is insufficient.
    pub required_signature: MethodContract,
    pub source_nominal_id: u32,
    pub dispatch_slot: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralMemberWitness {
    pub tenant_vm_id: u64,
    pub source_nominal_id: u32,
    pub layout_epoch: u64,
    pub region_loan_id: u64,
    pub fields: Vec<ReadFieldWitness>,
    pub methods: Vec<MethodWitness>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberError {
    Shape(ShapeError),
    UnsupportedKind,
    InvalidDeclaration,
    TooManyMethods,
    MissingMethod,
    InaccessibleMethod,
    IncompatibleMethod,
    AmbiguousMethod,
    UnverifiedDispatch,
    TenantBoundary,
    TupleArity,
    TupleElementType,
    MapElementType,
    MapPresenceUnproven,
}

impl From<ShapeError> for MemberError {
    fn from(err: ShapeError) -> Self { Self::Shape(err) }
}

fn declared_signature_ok(method: &MethodContract) -> bool {
    !method.name.is_empty()
        && method.name.len() <= 256
        && method.params.len() <= MAX_METHOD_PARAMETERS
        && method.parameter_passing.len() == method.params.len()
        && !method.effects.iter().any(String::is_empty)
        && !method.permissions.iter().any(String::is_empty)
        && !method.generic_bounds.iter().any(String::is_empty)
        && method.effects.len() <= MAX_STRUCTURAL_METHODS
        && method.permissions.len() <= MAX_STRUCTURAL_METHODS
        && method.generic_bounds.len() <= MAX_STRUCTURAL_METHODS
}

fn is_subset(actual: &[String], allowed: &[String]) -> bool {
    let allowed: BTreeSet<&str> = allowed.iter().map(String::as_str).collect();
    actual.iter().all(|name| allowed.contains(name.as_str()))
}

/// A method may accept *more* arguments and return a more precise type, but
/// never require more effects/permissions than the caller's expected contract.
/// Ownership and generic bounds remain exact until variance is formally proven.
fn compatible(actual: &MethodContract, required: &MethodContract)
    -> Result<bool, MemberError>
{
    if actual.name != required.name
        || actual.params.len() != required.params.len()
        || actual.is_async != required.is_async
        || actual.parameter_passing != required.parameter_passing
        || actual.result_passing != required.result_passing
        || actual.generic_bounds != required.generic_bounds
        || (required.receiver == ReceiverMode::Readonly
            && actual.receiver != ReceiverMode::Readonly)
        || !is_subset(&actual.effects, &required.effects)
        || !is_subset(&actual.permissions, &required.permissions)
    {
        return Ok(false);
    }
    for (expected_arg, actual_arg) in required.params.iter().zip(&actual.params) {
        if !readonly_subtype(expected_arg, actual_arg)? {
            return Ok(false);
        }
    }
    Ok(readonly_subtype(&actual.result, &required.result)?)
}

fn check_declarations(
    actual: &SourceMembers, required: &RequiredMembers,
) -> Result<(), MemberError> {
    if required.methods.len() > MAX_STRUCTURAL_METHODS
        || actual.methods.len() > MAX_STRUCTURAL_METHODS
    {
        return Err(MemberError::TooManyMethods);
    }
    if required.fields.len() > MAX_SHAPE_FIELDS
        || actual.shape.fields.len() > MAX_SHAPE_FIELDS
    {
        return Err(MemberError::Shape(ShapeError::TooManyFields));
    }
    // Even a method-only interface witness cannot accept malformed concrete
    // layout metadata. Otherwise no requested field would invoke the lower-
    // level shape validator and an unchecked receiver could be admitted.
    let mut field_names = BTreeSet::new();
    let mut offsets = BTreeSet::new();
    for field in &actual.shape.fields {
        if field.name.is_empty() || !field_names.insert(field.name.as_str()) {
            return Err(MemberError::Shape(ShapeError::DuplicateField));
        }
        if actual.shape.kind == SourceKind::Map {
            if field.actual_offset.is_some() {
                return Err(MemberError::Shape(ShapeError::UntrustedLayout));
            }
        } else {
            let offset = field.actual_offset.ok_or(
                MemberError::Shape(ShapeError::UntrustedLayout))?;
            if !offsets.insert(offset) {
                return Err(MemberError::Shape(ShapeError::UntrustedLayout));
            }
        }
    }
    if actual.shape.kind == SourceKind::Tuple && !actual.shape.fields.is_empty() {
        return Err(MemberError::InvalidDeclaration);
    }
    if required.kind == RequiredKind::Interface && !required.fields.is_empty() {
        return Err(MemberError::InvalidDeclaration);
    }
    if required.methods.iter().any(|m| !declared_signature_ok(m))
        || actual.methods.iter().any(|m| !declared_signature_ok(&m.signature))
    {
        return Err(MemberError::InvalidDeclaration);
    }
    // Duplicated required signatures would force an ambiguous member set.
    // Same name with distinct parameter types is an admissible overload.
    for (i, method) in required.methods.iter().enumerate() {
        if required.methods[..i].iter().any(|previous| {
            previous.name == method.name && previous.params == method.params
                && previous.parameter_passing == method.parameter_passing
        }) {
            return Err(MemberError::InvalidDeclaration);
        }
    }
    if actual.tenant_vm_id == 0 || required.tenant_vm_id == 0
        || actual.tenant_vm_id != required.tenant_vm_id
    {
        return Err(MemberError::TenantBoundary);
    }
    if actual.shape.sealed_actor_state {
        return Err(MemberError::Shape(ShapeError::SealedActorState));
    }
    if actual.shape.layout_epoch == 0 || actual.shape.region_loan_id == 0 {
        return Err(MemberError::Shape(ShapeError::UnprovenLoan));
    }
    Ok(())
}

/// Prepare a read-only, all-required-member structural witness.
///
/// This is **proof preparation**, not guest method execution. In particular,
/// a valid `MethodWitness` does not authorize an unchecked indirect call;
/// the lowering stage must validate the slot against the concrete type and
/// keep the source loan alive throughout the invocation.
pub fn prove_structural_members(
    actual: &SourceMembers,
    required: &RequiredMembers,
) -> Result<StructuralMemberWitness, MemberError> {
    check_declarations(actual, required)?;
    match required.kind {
        RequiredKind::Tuple => {
            // Tuple layout is positional and exact-arity, not row-width based.
            if actual.shape.kind != SourceKind::Tuple {
                return Err(MemberError::UnsupportedKind);
            }
            if actual.tuple_elements.len() != required.tuple_elements.len() {
                return Err(MemberError::TupleArity);
            }
            for (source, target) in actual.tuple_elements.iter().zip(&required.tuple_elements) {
                if !readonly_subtype(source, target)? {
                    return Err(MemberError::TupleElementType);
                }
            }
        }
        RequiredKind::Map => {
            if actual.shape.kind != SourceKind::Map {
                return Err(MemberError::UnsupportedKind);
            }
            if actual.map_elements.as_ref() != required.map_elements.as_ref()
                || required.map_elements.is_none()
            {
                return Err(MemberError::MapElementType);
            }
        }
        _ => {
            if !matches!(actual.shape.kind,
                SourceKind::Class | SourceKind::Struct | SourceKind::Record)
            {
                // Dynamic maps do not automatically satisfy interfaces or
                // named field shapes. An independently checked adapter is needed.
                return Err(MemberError::UnsupportedKind);
            }
        }
    }
    // An ordinary Map<K,V> cannot prove that a specific named key is present.
    if actual.shape.kind == SourceKind::Map && !required.fields.is_empty()
        && actual.shape.map_snapshot_id.is_none()
    {
        return Err(MemberError::MapPresenceUnproven);
    }
    let fields = if required.fields.is_empty() {
        Vec::new()
    } else {
        prove_readonly_projection(&actual.shape, &required.fields)?.fields
    };

    let mut witnesses = Vec::with_capacity(required.methods.len());
    for target in &required.methods {
        let named: Vec<&SourceMethod> = actual.methods.iter()
            .filter(|candidate| candidate.signature.name == target.name)
            .collect();
        if named.is_empty() {
            return Err(MemberError::MissingMethod);
        }
        let exposed: Vec<&SourceMethod> = named.iter().copied()
            .filter(|candidate| candidate.public && candidate.is_instance
                && !candidate.sealed_actor_state)
            .collect();
        if exposed.is_empty() {
            return Err(MemberError::InaccessibleMethod);
        }
        let mut applicable = Vec::new();
        for candidate in exposed {
            if compatible(&candidate.signature, target)? {
                applicable.push(candidate);
            }
        }
        if applicable.is_empty() {
            return Err(MemberError::IncompatibleMethod);
        }
        if applicable.len() != 1 {
            return Err(MemberError::AmbiguousMethod);
        }
        let slot = applicable[0].dispatch_slot.ok_or(MemberError::UnverifiedDispatch)?;
        witnesses.push(MethodWitness {
            name: target.name.clone(),
            required_signature: target.clone(),
            source_nominal_id: actual.shape.source_nominal_id,
            dispatch_slot: slot,
        });
    }
    witnesses.sort_by(|a,b| a.name.cmp(&b.name)
        .then(a.dispatch_slot.cmp(&b.dispatch_slot)));
    // One concrete source dispatch slot cannot secretly implement two
    // distinct required callable signatures without a verified alias thunk.
    let mut selected_slots = BTreeSet::new();
    for witness in &witnesses {
        if !selected_slots.insert(witness.dispatch_slot) {
            return Err(MemberError::UnverifiedDispatch);
        }
    }
    Ok(StructuralMemberWitness {
        tenant_vm_id: actual.tenant_vm_id,
        source_nominal_id: actual.shape.source_nominal_id,
        layout_epoch: actual.shape.layout_epoch,
        region_loan_id: actual.shape.region_loan_id,
        fields,
        methods: witnesses,
    })
}
