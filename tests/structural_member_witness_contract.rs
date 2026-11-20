//! Real Rust tests of a *non-emitting* structural member witness planner.
//! A passing test here is NOT proof that an Atto guest calls through ~Interface.
//! Executable method receiver ABI is intentionally a separate milestone.
use atto_compiler::shape_witness::{
    ReadAccess, RequiredField, ShapeError, ShapeTy, SourceField,
    SourceKind, SourceShape,
};
use atto_compiler::structural_member_witness::{
    prove_structural_members, MemberError, MethodContract, Passing,
    ReceiverMode, RequiredKind, RequiredMembers, SourceMembers, SourceMethod,
};

fn field(name: &str, ty: ShapeTy, offset: u32) -> SourceField {
    SourceField {
        name: name.into(), ty, public: true, readable: true,
        guaranteed_present: true, effect_free: true,
        sealed_actor_state: false, actual_offset: Some(offset),
    }
}

fn shape(kind: SourceKind) -> SourceShape {
    SourceShape {
        kind, source_nominal_id: 77, layout_epoch: 12,
        region_loan_id: 99, sealed_actor_state: false,
        map_snapshot_id: None, fields: vec![],
    }
}

fn actual(kind: SourceKind) -> SourceMembers {
    SourceMembers {
        tenant_vm_id: 7, shape: shape(kind), methods: vec![],
        tuple_elements: vec![], map_elements: None,
    }
}

fn required(kind: RequiredKind) -> RequiredMembers {
    RequiredMembers {
        kind, tenant_vm_id: 7, fields: vec![], methods: vec![],
        tuple_elements: vec![], map_elements: None,
    }
}

fn method(name: &str, params: Vec<ShapeTy>, result: ShapeTy) -> MethodContract {
    MethodContract {
        name: name.into(),
        parameter_passing: vec![Passing::Borrow; params.len()],
        params, result, receiver: ReceiverMode::Readonly,
        result_passing: Passing::Borrow, is_async: false,
        effects: vec![], permissions: vec![], generic_bounds: vec![],
    }
}

fn impl_method(signature: MethodContract, slot: u32) -> SourceMethod {
    SourceMethod {
        signature, public: true, is_instance: true,
        sealed_actor_state: false, dispatch_slot: Some(slot),
    }
}

#[test]
fn unrelated_class_satisfies_all_required_interface_methods() {
    let mut candidate = actual(SourceKind::Class);
    candidate.shape.source_nominal_id = 41;
    candidate.methods = vec![
        impl_method(method("read", vec![], ShapeTy::String), 1),
        impl_method(method("close", vec![], ShapeTy::Bool), 9),
        impl_method(method("extra", vec![], ShapeTy::Int), 12),
    ];
    let mut contract = required(RequiredKind::Interface);
    contract.methods = vec![
        method("close", vec![], ShapeTy::Bool),
        method("read", vec![], ShapeTy::String),
    ];
    let proof = prove_structural_members(&candidate, &contract).unwrap();
    assert_eq!(proof.source_nominal_id, 41);
    assert_eq!(proof.tenant_vm_id, 7);
    assert_eq!(proof.region_loan_id, 99);
    assert_eq!(proof.methods.len(), 2);
    assert_eq!(proof.methods[0].name, "close");
    assert_eq!(proof.methods[0].dispatch_slot, 9);
    assert_eq!(proof.methods[1].name, "read");
    assert_eq!(proof.methods[1].dispatch_slot, 1);
}

#[test]
fn a_partial_interface_does_not_conform() {
    let mut candidate = actual(SourceKind::Class);
    candidate.methods.push(impl_method(method("read", vec![], ShapeTy::String), 3));
    let mut contract = required(RequiredKind::Interface);
    contract.methods = vec![
        method("read", vec![], ShapeTy::String),
        method("close", vec![], ShapeTy::Bool),
    ];
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::MissingMethod));
}

#[test]
fn private_static_and_sealed_members_are_not_public_instance_methods() {
    let base = method("read", vec![], ShapeTy::String);
    let mut contract = required(RequiredKind::Interface);
    contract.methods.push(base.clone());
    let mut candidate = actual(SourceKind::Class);
    candidate.methods.push(impl_method(base, 3));
    candidate.methods[0].public = false;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::InaccessibleMethod));
    candidate.methods[0].public = true;
    candidate.methods[0].is_instance = false;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::InaccessibleMethod));
    candidate.methods[0].is_instance = true;
    candidate.methods[0].sealed_actor_state = true;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::InaccessibleMethod));
}

#[test]
fn stronger_effects_permissions_and_receiver_mutability_fail() {
    let mut contract = required(RequiredKind::Interface);
    contract.methods.push(method("read", vec![], ShapeTy::String));
    let mut candidate = actual(SourceKind::Class);
    candidate.methods.push(impl_method(contract.methods[0].clone(), 8));
    candidate.methods[0].signature.effects.push("throws".into());
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::IncompatibleMethod));
    candidate.methods[0].signature.effects.clear();
    candidate.methods[0].signature.permissions.push("net".into());
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::IncompatibleMethod));
    candidate.methods[0].signature.permissions.clear();
    candidate.methods[0].signature.receiver = ReceiverMode::Mutable;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::IncompatibleMethod));
}

#[test]
fn consuming_argument_or_async_mismatch_cannot_launder_borrow() {
    let mut contract = required(RequiredKind::Interface);
    contract.methods.push(method("inspect", vec![ShapeTy::Int], ShapeTy::Bool));
    let mut candidate = actual(SourceKind::Class);
    candidate.methods.push(impl_method(contract.methods[0].clone(), 6));
    candidate.methods[0].signature.parameter_passing[0] = Passing::Take;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::IncompatibleMethod));
    candidate.methods[0].signature.parameter_passing[0] = Passing::Borrow;
    candidate.methods[0].signature.is_async = true;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::IncompatibleMethod));
}

#[test]
fn duplicate_compatible_methods_are_ambiguous_and_unverified_slots_reject() {
    let m = method("run", vec![], ShapeTy::Int);
    let mut contract = required(RequiredKind::Trait);
    contract.methods.push(m.clone());
    let mut candidate = actual(SourceKind::Class);
    candidate.methods = vec![impl_method(m.clone(), 3), impl_method(m, 4)];
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::AmbiguousMethod));
    candidate.methods.pop();
    candidate.methods[0].dispatch_slot = None;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::UnverifiedDispatch));
}

#[test]
fn overloads_retain_distinct_required_signatures_and_slots() {
    let mut contract = required(RequiredKind::Interface);
    contract.methods.push(method("get", vec![ShapeTy::Int], ShapeTy::String));
    contract.methods.push(method("get", vec![ShapeTy::Bool], ShapeTy::String));
    let mut candidate = actual(SourceKind::Class);
    candidate.methods = vec![
        impl_method(contract.methods[1].clone(), 27),
        impl_method(contract.methods[0].clone(), 14),
    ];
    let proof = prove_structural_members(&candidate, &contract).unwrap();
    assert_eq!(proof.methods.len(), 2);
    assert_ne!(proof.methods[0].required_signature.params,
               proof.methods[1].required_signature.params);
    assert_ne!(proof.methods[0].dispatch_slot, proof.methods[1].dispatch_slot);
    contract.methods.push(contract.methods[0].clone());
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::InvalidDeclaration));
}

#[test]
fn safe_parameter_contravariance_and_return_covariance() {
    let mut contract = required(RequiredKind::Interface);
    contract.methods.push(method("echo",
        vec![ShapeTy::LiteralInt(5)], ShapeTy::String));
    let mut candidate = actual(SourceKind::Class);
    candidate.methods.push(impl_method(method("echo",
        vec![ShapeTy::Int], ShapeTy::LiteralString("ok".into())), 12));
    assert!(prove_structural_members(&candidate, &contract).is_ok());
    candidate.methods[0].signature.params[0] = ShapeTy::LiteralInt(3);
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::IncompatibleMethod));
}

#[test]
fn class_contract_combines_all_fields_and_methods_using_actual_offsets() {
    let mut contract = required(RequiredKind::Class);
    contract.fields = vec![
        RequiredField { name: "x".into(), ty: ShapeTy::Int },
        RequiredField { name: "y".into(), ty: ShapeTy::Int },
    ];
    contract.methods.push(method("distance", vec![], ShapeTy::Int));
    let mut candidate = actual(SourceKind::Struct);
    candidate.shape.fields = vec![
        field("y", ShapeTy::Int, 72),
        field("padding", ShapeTy::Bool, 8),
        field("x", ShapeTy::Int, 48),
    ];
    candidate.methods.push(impl_method(contract.methods[0].clone(), 2));
    let proof = prove_structural_members(&candidate, &contract).unwrap();
    assert_eq!(proof.fields[0].access, ReadAccess::SourceOffset(48));
    assert_eq!(proof.fields[1].access, ReadAccess::SourceOffset(72));
    candidate.methods.clear();
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::MissingMethod));
}

#[test]
fn trait_requires_its_public_field_and_method_both() {
    let mut contract = required(RequiredKind::Trait);
    contract.fields.push(RequiredField { name: "id".into(), ty: ShapeTy::Int });
    contract.methods.push(method("run", vec![], ShapeTy::Bool));
    let mut candidate = actual(SourceKind::Class);
    candidate.shape.fields.push(field("id", ShapeTy::Int, 24));
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::MissingMethod));
    candidate.methods.push(impl_method(contract.methods[0].clone(), 1));
    assert!(prove_structural_members(&candidate, &contract).is_ok());
    candidate.shape.fields[0].public = false;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::Shape(ShapeError::MissingOrInaccessibleField)));
}

#[test]
fn tuple_arity_and_positional_element_types_are_exact() {
    let mut contract = required(RequiredKind::Tuple);
    contract.tuple_elements = vec![ShapeTy::Int, ShapeTy::String];
    let mut candidate = actual(SourceKind::Tuple);
    candidate.tuple_elements = vec![ShapeTy::Int, ShapeTy::String];
    assert!(prove_structural_members(&candidate, &contract).is_ok());
    candidate.tuple_elements.push(ShapeTy::Bool);
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::TupleArity));
    candidate.tuple_elements = vec![ShapeTy::Int, ShapeTy::Bool];
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::TupleElementType));
    candidate.shape.kind = SourceKind::Class;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::UnsupportedKind));
}

#[test]
fn map_generic_key_type_is_not_a_presence_witness() {
    let mut contract = required(RequiredKind::Map);
    contract.map_elements = Some((ShapeTy::String, ShapeTy::Int));
    contract.fields.push(RequiredField { name: "foo".into(), ty: ShapeTy::Int });
    let mut candidate = actual(SourceKind::Map);
    candidate.map_elements = Some((ShapeTy::String, ShapeTy::Int));
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::MapPresenceUnproven));
    candidate.shape.map_snapshot_id = Some(91);
    candidate.shape.fields.push(SourceField {
        name:"foo".into(), ty:ShapeTy::Int, public:true,
        readable:true, guaranteed_present:true, effect_free:true,
        sealed_actor_state:false, actual_offset:None,
    });
    let witness = prove_structural_members(&candidate, &contract).unwrap();
    assert_eq!(witness.fields[0].access, ReadAccess::PlannedSnapshotKey {
        name:"foo".into(), snapshot_id:91,
    });
    candidate.map_elements = Some((ShapeTy::Int, ShapeTy::Int));
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::MapElementType));
}

#[test]
fn cross_tenant_and_stale_loans_cannot_be_witnessed() {
    let contract = required(RequiredKind::Interface);
    let mut candidate = actual(SourceKind::Class);
    candidate.tenant_vm_id = 8;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::TenantBoundary));
    candidate.tenant_vm_id = 7;
    candidate.shape.region_loan_id = 0;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::Shape(ShapeError::UnprovenLoan)));
    candidate.shape.region_loan_id = 99;
    candidate.shape.sealed_actor_state = true;
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::Shape(ShapeError::SealedActorState)));
}

#[test]
fn unsupported_map_as_interface_and_invalid_interface_data_reject() {
    let mut contract = required(RequiredKind::Interface);
    let candidate = actual(SourceKind::Map);
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::UnsupportedKind));
    contract.fields.push(RequiredField {name:"id".into(),ty:ShapeTy::Int});
    let candidate = actual(SourceKind::Class);
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::InvalidDeclaration));
}

#[test]
fn method_only_interface_cannot_bypass_concrete_layout_validation() {
    let contract = required(RequiredKind::Interface);
    let mut candidate = actual(SourceKind::Class);
    candidate.shape.fields = vec![
        field("secret", ShapeTy::Int, 16),
        field("secret", ShapeTy::Int, 24),
    ];
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::Shape(ShapeError::DuplicateField)));
    candidate.shape.fields[1].name = "other".into();
    candidate.shape.fields[1].actual_offset = Some(16);
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::Shape(ShapeError::UntrustedLayout)));
}

#[test]
fn two_distinct_required_methods_cannot_reuse_one_unverified_dispatch_slot() {
    let mut contract = required(RequiredKind::Interface);
    contract.methods = vec![
        method("read", vec![], ShapeTy::Int),
        method("close", vec![], ShapeTy::Bool),
    ];
    let mut candidate = actual(SourceKind::Class);
    candidate.methods = vec![
        impl_method(contract.methods[0].clone(), 13),
        impl_method(contract.methods[1].clone(), 13),
    ];
    assert_eq!(prove_structural_members(&candidate, &contract),
               Err(MemberError::UnverifiedDispatch));
}
