//! Compiler-owned read-only structural shape witnesses.
//!
//! This module is a deliberately non-emitting precursor to the projected
//! argument ABI. It never converts a class pointer to another nominal class,
//! manufactures write authority, invokes a getter, or transfers actor state.
//! The caller must supply *type-checked declaration metadata*, not guest data.
//!
//! In particular, a Map<K,V> key type does not prove that any key is present.
//! Map access plans carry a snapshot ID but are not executable until a runtime
//! snapshot/loan adapter validates and holds that snapshot.
use std::collections::BTreeSet;

pub const MAX_SHAPE_FIELDS: usize = 32;
pub const MAX_SHAPE_DEPTH: usize = 32;
pub const MAX_TYPE_VISITS: usize = 4096;
pub const MAX_LITERAL_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeTy {
    Int,
    Bool,
    Float,
    String,
    LiteralInt(i64),
    LiteralBool(bool),
    LiteralString(String),
    /// A union is a disjunction. Empty unions are rejected, not treated as Any.
    Union(Vec<ShapeTy>),
    /// Open, read-only row: additional accessible fields are permitted.
    Row(Vec<RequiredField>),
    /// Actor refs, handles, Any, unverified generics, etc. cannot be coerced.
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredField {
    pub name: String,
    pub ty: ShapeTy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Class,
    Record,
    Struct,
    /// Only a verified immutable snapshot/key witness can be projected.
    Map,
}

#[derive(Clone, Debug)]
pub struct SourceField {
    pub name: String,
    pub ty: ShapeTy,
    pub public: bool,
    pub readable: bool,
    pub guaranteed_present: bool,
    pub effect_free: bool,
    pub sealed_actor_state: bool,
    /// A byte offset from the ACTUAL source nominal type, not the target.
    /// All fixed-layout reads must use this offset after type checking.
    pub actual_offset: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct SourceShape {
    pub kind: SourceKind,
    pub source_nominal_id: u32,
    pub layout_epoch: u64,
    pub region_loan_id: u64,
    pub sealed_actor_state: bool,
    /// A future map adapter must issue this stable, scoped snapshot token.
    /// A nonzero value here is a proof *input*, not proof of atomicity itself.
    pub map_snapshot_id: Option<u64>,
    pub fields: Vec<SourceField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadAccess {
    SourceOffset(u32),
    /// No emitter supports this yet; requires snapshot lifetime and key read.
    PlannedSnapshotKey { name: String, snapshot_id: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadFieldWitness {
    pub name: String,
    pub ty: ShapeTy,
    pub access: ReadAccess,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadonlyProjection {
    /// Retain actual source identity. Do not substitute the expected nominal.
    pub source_nominal_id: u32,
    pub layout_epoch: u64,
    pub region_loan_id: u64,
    pub fields: Vec<ReadFieldWitness>,
}

impl ReadonlyProjection {
    pub fn field(&self, name: &str) -> Option<&ReadFieldWitness> {
        self.fields.iter().find(|field| field.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    EmptyShape,
    DuplicateField,
    TooManyFields,
    TypeBudget,
    UnsupportedType,
    MissingOrInaccessibleField,
    IncompatibleField,
    SealedActorState,
    UnprovenLoan,
    UnprovenMapSnapshot,
    UntrustedLayout,
}

/// True only for proven read-variance subtyping. Because a union on the
/// actual side means ALL possible actual alternatives, it must be checked
/// before the existential union on the expected side.
pub fn readonly_subtype(actual: &ShapeTy, expected: &ShapeTy) -> Result<bool, ShapeError> {
    let mut budget = MAX_TYPE_VISITS;
    subtype(actual, expected, 0, &mut budget)
}

fn subtype(
    actual: &ShapeTy,
    expected: &ShapeTy,
    depth: usize,
    budget: &mut usize,
) -> Result<bool, ShapeError> {
    if depth >= MAX_SHAPE_DEPTH || *budget == 0 {
        return Err(ShapeError::TypeBudget);
    }
    *budget -= 1;
    match (actual, expected) {
        (ShapeTy::Unverified, _) | (_, ShapeTy::Unverified) =>
            Err(ShapeError::UnsupportedType),
        (ShapeTy::Union(arms), _) => {
            if arms.is_empty() { return Err(ShapeError::EmptyShape); }
            for arm in arms {
                if !subtype(arm, expected, depth + 1, budget)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (_, ShapeTy::Union(arms)) => {
            if arms.is_empty() { return Err(ShapeError::EmptyShape); }
            for arm in arms {
                if subtype(actual, arm, depth + 1, budget)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        (ShapeTy::LiteralInt(_), ShapeTy::Int)
        | (ShapeTy::LiteralBool(_), ShapeTy::Bool)
        | (ShapeTy::LiteralString(_), ShapeTy::String) => {
            if let ShapeTy::LiteralString(v) = actual {
                if v.len() > MAX_LITERAL_BYTES { return Err(ShapeError::TypeBudget); }
            }
            Ok(true)
        }
        (ShapeTy::LiteralInt(a), ShapeTy::LiteralInt(b)) => Ok(a == b),
        (ShapeTy::LiteralBool(a), ShapeTy::LiteralBool(b)) => Ok(a == b),
        (ShapeTy::LiteralString(a), ShapeTy::LiteralString(b)) => {
            if a.len() > MAX_LITERAL_BYTES || b.len() > MAX_LITERAL_BYTES {
                return Err(ShapeError::TypeBudget);
            }
            Ok(a == b)
        }
        (ShapeTy::Row(actual), ShapeTy::Row(expected)) => {
            validate_fields(actual)?;
            validate_fields(expected)?;
            for requested in expected {
                let Some(present) = actual.iter().find(|f| f.name == requested.name) else {
                    return Ok(false);
                };
                if !subtype(&present.ty, &requested.ty, depth + 1, budget)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ShapeTy::Int, ShapeTy::Int) | (ShapeTy::Bool, ShapeTy::Bool)
        | (ShapeTy::String, ShapeTy::String) | (ShapeTy::Float, ShapeTy::Float) => Ok(true),
        _ => Ok(false),
    }
}

fn validate_fields(fields: &[RequiredField]) -> Result<(), ShapeError> {
    if fields.is_empty() { return Err(ShapeError::EmptyShape); }
    if fields.len() > MAX_SHAPE_FIELDS { return Err(ShapeError::TooManyFields); }
    let mut seen = BTreeSet::new();
    for field in fields {
        if field.name.is_empty() || !seen.insert(field.name.as_str()) {
            return Err(ShapeError::DuplicateField);
        }
    }
    Ok(())
}

/// Build a non-emitting, borrowed field-access plan from the *actual* source
/// layout. This is proof preparation, NOT permission to emit a cross-nominal
/// function call. Cross-class ABI lowering must consume verified offset
/// witnesses rather than reinterpreting a source pointer as the target type.
pub fn prove_readonly_projection(
    source: &SourceShape,
    requirements: &[RequiredField],
) -> Result<ReadonlyProjection, ShapeError> {
    validate_fields(requirements)?;
    if source.fields.len() > MAX_SHAPE_FIELDS {
        return Err(ShapeError::TooManyFields);
    }
    if source.sealed_actor_state { return Err(ShapeError::SealedActorState); }
    if source.layout_epoch == 0 || source.region_loan_id == 0 {
        return Err(ShapeError::UnprovenLoan);
    }
    let snapshot_id = if source.kind == SourceKind::Map {
        match source.map_snapshot_id {
            Some(v) if v != 0 => Some(v),
            _ => return Err(ShapeError::UnprovenMapSnapshot),
        }
    } else {
        if source.map_snapshot_id.is_some() { return Err(ShapeError::UntrustedLayout); }
        None
    };
    let mut field_names = BTreeSet::new();
    let mut offsets = BTreeSet::new();
    for field in &source.fields {
        if field.name.is_empty() || !field_names.insert(field.name.as_str()) {
            return Err(ShapeError::DuplicateField);
        }
        if source.kind == SourceKind::Map {
            if field.actual_offset.is_some() {
                return Err(ShapeError::UntrustedLayout);
            }
        } else {
            let offset = field.actual_offset.ok_or(ShapeError::UntrustedLayout)?;
            if !offsets.insert(offset) {
                return Err(ShapeError::UntrustedLayout);
            }
        }
    }
    let mut result = Vec::with_capacity(requirements.len());
    for required in requirements {
        let Some(field) = source.fields.iter().find(|f| f.name == required.name) else {
            return Err(ShapeError::MissingOrInaccessibleField);
        };
        if field.sealed_actor_state { return Err(ShapeError::SealedActorState); }
        if !field.public || !field.readable || !field.guaranteed_present
            || !field.effect_free {
            return Err(ShapeError::MissingOrInaccessibleField);
        }
        if !readonly_subtype(&field.ty, &required.ty)? {
            return Err(ShapeError::IncompatibleField);
        }
        let access = match snapshot_id {
            Some(id) => ReadAccess::PlannedSnapshotKey {
                name: field.name.clone(), snapshot_id: id,
            },
            None => ReadAccess::SourceOffset(
                field.actual_offset.ok_or(ShapeError::UntrustedLayout)?
            ),
        };
        result.push(ReadFieldWitness {
            name: required.name.clone(),
            ty: required.ty.clone(),
            access,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ReadonlyProjection {
        source_nominal_id: source.source_nominal_id,
        layout_epoch: source.layout_epoch,
        region_loan_id: source.region_loan_id,
        fields: result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required(name: &str, ty: ShapeTy) -> RequiredField {
        RequiredField { name: name.into(), ty }
    }

    fn scalar(name: &str, ty: ShapeTy, offset: u32) -> SourceField {
        SourceField {
            name: name.into(), ty, public: true, readable: true,
            guaranteed_present: true, effect_free: true,
            sealed_actor_state: false, actual_offset: Some(offset),
        }
    }

    fn source(kind: SourceKind) -> SourceShape {
        SourceShape {
            kind, source_nominal_id: 7, layout_epoch: 1, region_loan_id: 22,
            sealed_actor_state: false, map_snapshot_id: None,
            fields: vec![scalar("foo", ShapeTy::String, 24),
                         scalar("extra", ShapeTy::Int, 8)],
        }
    }

    #[test]
    fn class_layout_reordering_uses_actual_offset_not_target_offset() {
        let proof = prove_readonly_projection(
            &source(SourceKind::Class),
            &[required("foo", ShapeTy::String)],
        ).unwrap();
        assert_eq!(proof.source_nominal_id, 7);
        assert_eq!(proof.region_loan_id, 22);
        assert_eq!(proof.field("foo").unwrap().access, ReadAccess::SourceOffset(24));
        assert!(proof.field("extra").is_none());
    }

    #[test]
    fn record_and_struct_are_distinct_sources_with_checked_read_offsets() {
        for kind in [SourceKind::Record, SourceKind::Struct] {
            let p = prove_readonly_projection(
                &source(kind), &[required("extra", ShapeTy::Int)]
            ).unwrap();
            assert_eq!(p.field("extra").unwrap().access, ReadAccess::SourceOffset(8));
        }
    }

    #[test]
    fn source_extras_and_request_order_never_change_the_readonly_witness() {
        let src = source(SourceKind::Class);
        let a = prove_readonly_projection(&src, &[
            required("foo", ShapeTy::String), required("extra", ShapeTy::Int),
        ]).unwrap();
        let b = prove_readonly_projection(&src, &[
            required("extra", ShapeTy::Int), required("foo", ShapeTy::String),
        ]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn actual_union_has_to_fit_every_arm_of_expected_union() {
        let both = ShapeTy::Union(vec![ShapeTy::Int, ShapeTy::String]);
        assert_eq!(readonly_subtype(&both, &both), Ok(true));
        assert_eq!(readonly_subtype(&both, &ShapeTy::String), Ok(false));
        assert_eq!(readonly_subtype(&ShapeTy::String, &both), Ok(true));
        assert_eq!(readonly_subtype(&ShapeTy::Union(vec![]), &both),
                   Err(ShapeError::EmptyShape));
    }

    #[test]
    fn literal_fields_are_covariant_but_not_guaranteed_by_broad_field_types() {
        assert_eq!(readonly_subtype(
            &ShapeTy::LiteralString("dog".into()), &ShapeTy::String), Ok(true));
        assert_eq!(readonly_subtype(
            &ShapeTy::String, &ShapeTy::LiteralString("dog".into())), Ok(false));
        assert_eq!(readonly_subtype(
            &ShapeTy::LiteralString("dog".into()),
            &ShapeTy::LiteralString("cat".into())), Ok(false));
    }

    #[test]
    fn nested_rows_are_open_and_check_all_requested_fields() {
        let a = ShapeTy::Row(vec![
            required("y", ShapeTy::Int), required("x", ShapeTy::String)
        ]);
        let b = ShapeTy::Row(vec![required("x", ShapeTy::String)]);
        assert_eq!(readonly_subtype(&a, &b), Ok(true));
        assert_eq!(readonly_subtype(&b, &a), Ok(false));
    }

    #[test]
    fn private_fields_and_actor_state_are_never_projected() {
        let mut private = source(SourceKind::Class);
        private.fields[0].public = false;
        assert_eq!(prove_readonly_projection(
            &private, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::MissingOrInaccessibleField));
        let mut sealed = source(SourceKind::Class);
        sealed.sealed_actor_state = true;
        assert_eq!(prove_readonly_projection(
            &sealed, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::SealedActorState));
        let mut field_sealed = source(SourceKind::Class);
        field_sealed.fields[0].sealed_actor_state = true;
        assert_eq!(prove_readonly_projection(
            &field_sealed, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::SealedActorState));
    }

    #[test]
    fn getters_and_absent_fields_cannot_be_passed_as_readonly_members() {
        let mut src = source(SourceKind::Class);
        src.fields[0].effect_free = false;
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::MissingOrInaccessibleField));
        src.fields[0].effect_free = true;
        src.fields[0].guaranteed_present = false;
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::MissingOrInaccessibleField));
    }

    #[test]
    fn map_key_type_alone_is_not_a_presence_witness() {
        let mut map = source(SourceKind::Map);
        map.fields[0].actual_offset = None;
        map.fields[1].actual_offset = None;
        assert_eq!(prove_readonly_projection(
            &map, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::UnprovenMapSnapshot));
        map.map_snapshot_id = Some(5);
        map.fields[0].guaranteed_present = false;
        assert_eq!(prove_readonly_projection(
            &map, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::MissingOrInaccessibleField));
        map.fields[0].guaranteed_present = true;
        let witness = prove_readonly_projection(
            &map, &[required("foo", ShapeTy::String)]).unwrap();
        assert_eq!(witness.field("foo").unwrap().access,
            ReadAccess::PlannedSnapshotKey {
                name: "foo".into(), snapshot_id: 5,
            });
    }

    #[test]
    fn mismatched_fields_and_unverified_capabilities_fail_closed() {
        let src = source(SourceKind::Class);
        assert_eq!(prove_readonly_projection(
            &src, &[required("missing", ShapeTy::Bool)]),
            Err(ShapeError::MissingOrInaccessibleField));
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::Int)]),
            Err(ShapeError::IncompatibleField));
        assert_eq!(readonly_subtype(&ShapeTy::Unverified, &ShapeTy::Unverified),
                   Err(ShapeError::UnsupportedType));
    }

    #[test]
    fn duplicate_fields_and_overlapping_actual_offsets_are_rejected() {
        let mut src = source(SourceKind::Class);
        src.fields.push(scalar("foo", ShapeTy::String, 32));
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::DuplicateField));
        src.fields.pop();
        src.fields[1].actual_offset = Some(24);
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::UntrustedLayout));
        assert_eq!(prove_readonly_projection(
            &source(SourceKind::Class), &[
                required("foo", ShapeTy::String),
                required("foo", ShapeTy::String),
            ]), Err(ShapeError::DuplicateField));
    }

    #[test]
    fn missing_loan_epoch_and_wrong_layout_reject_before_lowering() {
        let mut src = source(SourceKind::Class);
        src.region_loan_id = 0;
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::UnprovenLoan));
        src.region_loan_id = 1;
        src.layout_epoch = 0;
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::UnprovenLoan));
        src.layout_epoch = 1;
        src.map_snapshot_id = Some(42);
        assert_eq!(prove_readonly_projection(
            &src, &[required("foo", ShapeTy::String)]),
            Err(ShapeError::UntrustedLayout));
    }

    #[test]
    fn depth_and_literal_budgets_protect_untrusted_type_shapes() {
        let long = ShapeTy::LiteralString("x".repeat(MAX_LITERAL_BYTES + 1));
        assert_eq!(readonly_subtype(&long, &ShapeTy::String),
                   Err(ShapeError::TypeBudget));
        let mut nested = ShapeTy::Int;
        for _ in 0..(MAX_SHAPE_DEPTH + 3) {
            nested = ShapeTy::Row(vec![required("next", nested)]);
        }
        assert_eq!(readonly_subtype(&nested, &nested),
                   Err(ShapeError::TypeBudget));
        let wide: Vec<_> = (0..(MAX_SHAPE_FIELDS + 1))
            .map(|i| required(&format!("f{i}"), ShapeTy::Int)).collect();
        assert_eq!(readonly_subtype(&ShapeTy::Row(wide.clone()),
                                   &ShapeTy::Row(wide)),
                   Err(ShapeError::TooManyFields));
    }

    #[test]
    fn readonly_projections_expose_no_mutating_or_ownership_consuming_api() {
        let p = prove_readonly_projection(
            &source(SourceKind::Class), &[required("foo", ShapeTy::String)]
        ).unwrap();
        assert!(matches!(p.field("foo").unwrap().access, ReadAccess::SourceOffset(24)));
        // Only a read plan exists: no mutable pointer, setter or move operator
        // is exposed. The future ABI must preserve the region_loan_id.
        assert_eq!(p.region_loan_id, 22);
    }
}
