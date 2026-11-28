//! These are typed descriptor *admission-model* tests, not compiled Atto guests.
use atto_compiler::copy_graph_v1::{
    admit_copy, CopyMode, CopyRequest, Error, Field, FieldKind, TypeDescriptor,
};

fn ty(id: u32, parent: Option<u32>, fields: Vec<Field>) -> TypeDescriptor {
    TypeDescriptor {
        id,
        parent,
        fields,
        deep_copy_witness: true,
        shallow_copy_witness: true,
        sendable: true,
    }
}
fn field(id: u32, kind: FieldKind) -> Field {
    Field { id, kind }
}
fn request(concrete_type: u32, static_type: u32, mode: CopyMode, actor_send: bool) -> CopyRequest {
    CopyRequest {
        concrete_type,
        static_type,
        mode,
        actor_send,
    }
}
fn err(types: &[TypeDescriptor], req: CopyRequest, code: &'static str) {
    assert_eq!(admit_copy(types, req), Err(Error(code)));
}

#[test]
fn inherited_private_and_child_fields_are_in_concrete_aot_plan() {
    // 1 is Base with a private owned field 3; 2 is Derived with
    // its own private owned field 4. Field IDs never expose visibility.
    let descriptors = [
        ty(1, None, vec![field(11, FieldKind::OwnedMutable(3))]),
        ty(2, Some(1), vec![field(22, FieldKind::OwnedMutable(4))]),
        ty(3, None, vec![field(30, FieldKind::Scalar)]),
        ty(4, None, vec![field(40, FieldKind::Scalar)]),
    ];
    let proof = admit_copy(&descriptors, request(2, 1, CopyMode::Deep, true)).unwrap();
    assert_eq!(proof.concrete_type, 2);
    assert_eq!(proof.copied_type_ids, vec![1, 2, 3, 4]);
    assert!(proof.actor_send_checked);
}

#[test]
fn deep_copyability_does_not_imply_shallow_copyability() {
    let types = [
        ty(1, None, vec![field(7, FieldKind::OwnedMutable(2))]),
        ty(2, None, vec![]),
    ];
    err(
        &types,
        request(1, 1, CopyMode::Shallow, false),
        "E_COPY_SHALLOW_ALIAS",
    );
    admit_copy(&types, request(1, 1, CopyMode::Deep, false)).unwrap();
}

#[test]
fn ordinary_mutable_sharing_is_not_a_shallow_readonly_loan_witness() {
    let types = [
        ty(1, None, vec![field(1, FieldKind::SharedMutable(2))]),
        ty(2, None, vec![]),
    ];
    err(
        &types,
        request(1, 1, CopyMode::Shallow, false),
        "E_COPY_SHALLOW_ALIAS",
    );
    err(
        &types,
        request(1, 1, CopyMode::Shallow, true),
        "E_COPY_ACTOR_ISOLATION",
    );
    admit_copy(&types, request(1, 1, CopyMode::Deep, true)).unwrap();
}

#[test]
fn a_sendable_proof_is_independent_of_a_deep_copy_witness() {
    let mut non_sendable = ty(2, None, vec![]);
    non_sendable.sendable = false;
    let types = [
        ty(1, None, vec![field(1, FieldKind::OwnedMutable(2))]),
        non_sendable,
    ];
    admit_copy(&types, request(1, 1, CopyMode::Deep, false)).unwrap();
    err(
        &types,
        request(1, 1, CopyMode::Deep, true),
        "E_COPY_SENDABLE",
    );
}

#[test]
fn unknown_concrete_type_or_missing_nested_witness_fails_closed() {
    let types = [ty(1, None, vec![])];
    err(
        &types,
        request(2, 1, CopyMode::Deep, true),
        "E_COPY_AOT_WITNESS",
    );
    let unknown_field = [ty(1, None, vec![field(1, FieldKind::OwnedMutable(99))])];
    err(
        &unknown_field,
        request(1, 1, CopyMode::Deep, false),
        "E_COPY_AOT_WITNESS",
    );
}

#[test]
fn base_typed_dispatch_must_preserve_actual_concrete_subtype() {
    let types = [
        ty(1, None, vec![]),
        ty(2, Some(1), vec![]),
        ty(3, None, vec![]),
    ];
    admit_copy(&types, request(2, 1, CopyMode::Deep, false)).unwrap();
    err(
        &types,
        request(3, 1, CopyMode::Deep, false),
        "E_COPY_LAYOUT",
    );
}

#[test]
fn no_copy_ffi_resource_and_loan_views_reject() {
    for (kind, expected) in [
        (FieldKind::Resource, "E_COPY_FORBIDDEN"),
        (FieldKind::Borrowed, "E_COPY_VIEW"),
        (FieldKind::Unresolved, "E_COPY_UNPROVEN"),
    ] {
        let types = [ty(1, None, vec![field(1, kind)])];
        err(&types, request(1, 1, CopyMode::Deep, true), expected);
    }
}

#[test]
fn positive_annotation_alone_does_not_grant_copy_witness() {
    let mut ty1 = ty(1, None, vec![]);
    ty1.deep_copy_witness = false;
    err(
        &[ty1],
        request(1, 1, CopyMode::Deep, false),
        "E_COPY_AOT_WITNESS",
    );
}

#[test]
fn recursive_type_graph_is_admitted_for_compiler_memoized_cloner() {
    let types = [ty(1, None, vec![field(2, FieldKind::OwnedMutable(1))])];
    let proof = admit_copy(&types, request(1, 1, CopyMode::Deep, true)).unwrap();
    assert_eq!(proof.copied_type_ids, vec![1]);
}

#[test]
fn malformed_private_field_inventory_rejects_layout_duplicates() {
    let types = [ty(
        1,
        None,
        vec![field(1, FieldKind::Scalar), field(1, FieldKind::Scalar)],
    )];
    err(
        &types,
        request(1, 1, CopyMode::Deep, false),
        "E_COPY_LAYOUT",
    );
    let dup = [ty(1, None, vec![]), ty(1, None, vec![])];
    err(&dup, request(1, 1, CopyMode::Deep, false), "E_COPY_LAYOUT");
}

#[test]
fn cyclic_nominal_inheritance_is_not_legal_polymorphic_dispatch() {
    let types = [ty(1, Some(2), vec![]), ty(2, Some(1), vec![])];
    // The explicit Base static type 3 cannot be reached from 1, and the
    // preflight must reject the inheritance cycle.
    err(
        &types,
        request(1, 3, CopyMode::Deep, false),
        "E_COPY_LAYOUT",
    );
}

#[test]
fn immutable_frozen_field_permits_actor_send_with_independent_root() {
    let types = [ty(1, None, vec![field(1, FieldKind::ImmutableShared)])];
    admit_copy(&types, request(1, 1, CopyMode::Shallow, true)).unwrap();
    admit_copy(&types, request(1, 1, CopyMode::Deep, true)).unwrap();
}

#[test]
fn inherited_mutable_sharing_cannot_bypass_shallow_admission() {
    let types = [
        ty(1, None, vec![field(1, FieldKind::SharedMutable(3))]),
        ty(2, Some(1), vec![field(2, FieldKind::Scalar)]),
        ty(3, None, vec![field(3, FieldKind::Scalar)]),
    ];
    err(
        &types,
        request(2, 1, CopyMode::Shallow, false),
        "E_COPY_SHALLOW_ALIAS",
    );
    let plan = admit_copy(&types, request(2, 1, CopyMode::Deep, false)).unwrap();
    assert_eq!(plan.copied_type_ids, vec![1, 2, 3]);
}

#[test]
fn shallow_self_cycle_needs_readonly_transition_not_a_type_witness() {
    let types = [ty(1, None, vec![field(1, FieldKind::SharedMutable(1))])];
    err(
        &types,
        request(1, 1, CopyMode::Shallow, false),
        "E_COPY_SHALLOW_ALIAS",
    );
    assert_eq!(
        admit_copy(&types, request(1, 1, CopyMode::Deep, false))
            .unwrap()
            .copied_type_ids,
        vec![1],
    );
}

#[test]
fn unknown_shared_child_is_not_a_valid_shallow_descriptor() {
    for nested in [0, 99] {
        let types = [ty(
            1,
            None,
            vec![field(1, FieldKind::SharedMutable(nested))],
        )];
        err(
            &types,
            request(1, 1, CopyMode::Shallow, false),
            "E_COPY_AOT_WITNESS",
        );
    }
}

#[test]
fn flat_scalar_and_immutable_backing_remain_shallow_copyable() {
    let types = [
        ty(1, None, vec![field(1, FieldKind::Scalar)]),
        ty(2, Some(1), vec![field(2, FieldKind::ImmutableShared)]),
    ];
    let plan = admit_copy(&types, request(2, 1, CopyMode::Shallow, false)).unwrap();
    assert_eq!(plan.copied_type_ids, vec![1, 2]);
}
