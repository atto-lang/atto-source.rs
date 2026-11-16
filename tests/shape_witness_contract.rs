//! Integration-level non-emitting witness contract.
//! These tests require Rust execution. They do not compile an Atto guest or
//! authorize cross-nominal pointer casts / mutable structural writes.
use atto_compiler::shape_witness::{
    prove_readonly_projection, readonly_subtype, ReadAccess, RequiredField,
    ShapeError, ShapeTy, SourceField, SourceKind, SourceShape,
};

fn f(name: &str, ty: ShapeTy, offset: u32) -> SourceField {
    SourceField {
        name: name.into(), ty, public: true, readable: true,
        guaranteed_present: true, effect_free: true,
        sealed_actor_state: false, actual_offset: Some(offset),
    }
}

fn class(id: u32, fields: Vec<SourceField>) -> SourceShape {
    SourceShape {
        kind: SourceKind::Class, source_nominal_id: id,
        layout_epoch: 4, region_loan_id: 11,
        sealed_actor_state: false, map_snapshot_id: None, fields,
    }
}

fn req(name: &str, ty: ShapeTy) -> RequiredField {
    RequiredField { name: name.into(), ty }
}

#[test]
fn two_nominal_classes_with_same_public_shape_keep_different_real_offsets() {
    let a = class(10, vec![f("x", ShapeTy::Int, 8), f("y", ShapeTy::Int, 16)]);
    let b = class(11, vec![f("y", ShapeTy::Int, 56), f("x", ShapeTy::Int, 32)]);
    let requirements = [req("x", ShapeTy::Int), req("y", ShapeTy::Int)];
    let wa = prove_readonly_projection(&a, &requirements).unwrap();
    let wb = prove_readonly_projection(&b, &requirements).unwrap();
    assert_eq!(wa.source_nominal_id, 10);
    assert_eq!(wb.source_nominal_id, 11);
    assert_eq!(wa.field("x").unwrap().access, ReadAccess::SourceOffset(8));
    assert_eq!(wb.field("x").unwrap().access, ReadAccess::SourceOffset(32));
    assert_ne!(wa.field("x"), wb.field("x"));
}

#[test]
fn singleton_field_is_safe_to_read_as_a_general_string() {
    let a = class(1, vec![f("foo", ShapeTy::LiteralString("dog".into()), 8)]);
    assert!(prove_readonly_projection(&a, &[req("foo", ShapeTy::String)]).is_ok());
    let b = class(2, vec![f("foo", ShapeTy::String, 8)]);
    assert_eq!(prove_readonly_projection(&b, &[
        req("foo", ShapeTy::LiteralString("dog".into())),
    ]), Err(ShapeError::IncompatibleField));
}

#[test]
fn union_field_is_read_covariant_but_requires_narrowing() {
    let combined = ShapeTy::Union(vec![ShapeTy::String, ShapeTy::Int]);
    let a = class(1, vec![f("foo", combined.clone(), 8)]);
    assert!(prove_readonly_projection(&a, &[req("foo", combined.clone())]).is_ok());
    assert_eq!(prove_readonly_projection(&a, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::IncompatibleField));
    assert_eq!(readonly_subtype(&ShapeTy::Int, &combined), Ok(true));
}

#[test]
fn protected_or_actor_state_fields_are_never_witnessed() {
    let mut a = class(1, vec![f("foo", ShapeTy::Int, 8)]);
    a.fields[0].public = false;
    assert_eq!(prove_readonly_projection(&a, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::MissingOrInaccessibleField));
    a.fields[0].public = true;
    a.fields[0].sealed_actor_state = true;
    assert_eq!(prove_readonly_projection(&a, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::SealedActorState));
}

#[test]
fn map_lookup_is_non_executable_without_a_verified_snapshot_token() {
    let mut map = class(1, vec![f("foo", ShapeTy::Int, 8)]);
    map.kind = SourceKind::Map;
    map.fields[0].actual_offset = None;
    assert_eq!(prove_readonly_projection(&map, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::UnprovenMapSnapshot));
    map.map_snapshot_id = Some(6);
    map.fields[0].guaranteed_present = false;
    assert_eq!(prove_readonly_projection(&map, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::MissingOrInaccessibleField));
}

#[test]
fn missing_borrow_identity_and_stale_layout_generation_are_fail_closed() {
    let mut a = class(1, vec![f("foo", ShapeTy::Int, 8)]);
    a.region_loan_id = 0;
    assert_eq!(prove_readonly_projection(&a, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::UnprovenLoan));
    a.region_loan_id = 1;
    a.layout_epoch = 0;
    assert_eq!(prove_readonly_projection(&a, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::UnprovenLoan));
}

#[test]
fn duplicate_source_fields_and_target_fields_cannot_launder_a_projection() {
    let mut a = class(1, vec![f("foo", ShapeTy::Int, 8)]);
    a.fields.push(f("foo", ShapeTy::Int, 16));
    assert_eq!(prove_readonly_projection(&a, &[req("foo", ShapeTy::Int)]),
               Err(ShapeError::DuplicateField));
    a.fields.pop();
    assert_eq!(prove_readonly_projection(&a, &[
        req("foo", ShapeTy::Int), req("foo", ShapeTy::Int),
    ]), Err(ShapeError::DuplicateField));
}

#[test]
fn fixed_layout_is_never_inferred_from_a_generic_map_key_type() {
    let mut map = class(1, vec![f("foo", ShapeTy::String, 8)]);
    map.kind = SourceKind::Map;
    map.map_snapshot_id = Some(1);
    assert_eq!(prove_readonly_projection(&map, &[req("foo", ShapeTy::String)]),
               Err(ShapeError::UntrustedLayout));
    map.fields[0].actual_offset = None;
    let proof = prove_readonly_projection(&map, &[req("foo", ShapeTy::String)]).unwrap();
    assert_eq!(proof.field("foo").unwrap().access,
               ReadAccess::PlannedSnapshotKey {
                   name: "foo".into(), snapshot_id: 1,
               });
}
