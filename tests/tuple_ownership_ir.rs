//! Host-side typed loan-plan tests. These are not compiled Atto guest tests.
use atto_compiler::tuple_ownership_ir::{
    ElementType, LoanKind, ProjectionOp, Scalar, TupleOwner, TupleType,
};
use atto_compiler::tuple_binding_admission::{BindingMode, TupleMember};
use atto_compiler::Pos;

fn pos() -> Pos { Pos { line: 1, col: 1 } }
fn object(id: u32, name: &str) -> ElementType {
    ElementType::OwnedClass { class_id: id, name: name.into() }
}
fn scalar(s: Scalar) -> ElementType { ElementType::Copy(s) }
fn shape(elements: Vec<ElementType>) -> TupleType { TupleType::new(elements, pos()).unwrap() }
fn sample() -> TupleType {
    shape(vec![object(7, "Cell"), scalar(Scalar::Int), object(8, "Other")])
}
fn owner(mode: BindingMode) -> TupleOwner {
    TupleOwner::new(sample(), mode, false, pos()).unwrap()
}
fn binding(name: &str, mode: BindingMode, mutable: bool) -> TupleMember {
    TupleMember { name: Some(name.into()), mode, mutable_referent: mutable, annotation: None }
}
fn discarded() -> TupleMember {
    TupleMember { name: None, mode: BindingMode::Let, mutable_referent: false, annotation: None }
}
fn code<T>(outcome: Result<T, atto_compiler::Diagnostic>) -> &'static str {
    outcome.err().expect("expected rejection").code
}

#[test]
fn noncopy_projection_is_rooted_shared_loan_by_default() {
    let mut o = owner(BindingMode::Let);
    let p = o.project(&[
        binding("cell", BindingMode::Let, false),
        binding("count", BindingMode::Val, false),
        binding("other", BindingMode::Let, false),
    ], 2, pos()).unwrap();
    assert_eq!(p.len(), 3);
    assert!(matches!(p[0].operation, ProjectionOp::Borrow {kind: LoanKind::Shared, ..}));
    assert_eq!(p[1].operation, ProjectionOp::CopyValue);
    assert!(matches!(p[2].operation, ProjectionOp::Borrow {kind: LoanKind::Shared, ..}));
    assert_eq!(o.active_loans(), 2);
}
#[test]
fn exclusive_loans_of_disjoint_fields_can_coexist() {
    let mut o = owner(BindingMode::Let);
    let first = o.borrow_field(0, LoanKind::Exclusive, 1, pos()).unwrap();
    let second = o.borrow_field(2, LoanKind::Exclusive, 1, pos()).unwrap();
    assert_ne!(first, second);
    assert_eq!(o.active_loans(), 2);
    assert_eq!(code(o.borrow_field(0, LoanKind::Shared, 2, pos())), "E_BORROW");
}
#[test]
fn multiple_shared_loans_allowed_but_exclusive_is_not() {
    let mut o = owner(BindingMode::Let);
    o.borrow_field(0, LoanKind::Shared, 1, pos()).unwrap();
    o.borrow_field(0, LoanKind::Shared, 2, pos()).unwrap();
    assert_eq!(code(o.borrow_field(0, LoanKind::Exclusive, 2, pos())), "E_BORROW");
}
#[test]
fn root_loan_conflicts_with_overlapping_exclusive_element_loan() {
    let mut o = owner(BindingMode::Let);
    o.borrow_root(LoanKind::Shared, 1, pos()).unwrap();
    assert_eq!(code(o.borrow_field(2, LoanKind::Exclusive, 2, pos())), "E_BORROW");
    assert!(o.borrow_field(2, LoanKind::Shared, 2, pos()).is_ok());
    assert_eq!(code(o.borrow_root(LoanKind::Exclusive, 3, pos())), "E_BORROW");
}
#[test]
fn root_exclusive_loan_blocks_copy_scalar_reads() {
    let mut o = owner(BindingMode::Let);
    o.borrow_root(LoanKind::Exclusive, 1, pos()).unwrap();
    assert_eq!(code(o.read_copy(1, pos())), "E_BORROW");
}
#[test]
fn root_rebind_forbidden_until_loans_expire_then_allowed() {
    let mut o = owner(BindingMode::Let);
    let shape = o.shape().clone();
    let loan = o.borrow_field(0, LoanKind::Shared, 3, pos()).unwrap();
    assert_eq!(code(o.rebind_root(&shape, pos())), "E_BORROW");
    o.end_scope(3);
    assert_eq!(o.active_loans(), 0);
    assert!(o.rebind_root(&shape, pos()).is_ok());
    assert_eq!(code(o.end_loan(loan, pos())), "E_BORROW");
    let new_loan = o.borrow_field(0, LoanKind::Shared, 4, pos()).unwrap();
    assert_ne!(loan, new_loan);
}
#[test]
fn val_and_const_roots_cannot_rebind() {
    let o = owner(BindingMode::Val);
    let mut val = o.clone();
    assert_eq!(code(val.rebind_root(o.shape(), pos())), "E_IMMUTABLE");
    let shape = shape(vec![scalar(Scalar::Int)]);
    let mut konst = TupleOwner::new(shape.clone(), BindingMode::Const, true, pos()).unwrap();
    assert_eq!(code(konst.rebind_root(&shape, pos())), "E_IMMUTABLE");
}
#[test]
fn const_object_tuple_and_runtime_const_rejected() {
    assert_eq!(code(TupleOwner::new(sample(), BindingMode::Const, true, pos())), "E_CONST");
    let scalars = shape(vec![scalar(Scalar::Int)]);
    assert_eq!(code(TupleOwner::new(scalars, BindingMode::Const, false, pos())), "E_CONST");
}
#[test]
fn taking_a_loaned_field_is_rejected_without_moving_it() {
    let mut o = owner(BindingMode::Let);
    let loan = o.borrow_field(0, LoanKind::Shared, 1, pos()).unwrap();
    assert_eq!(code(o.take_field(0, pos())), "E_BORROW");
    assert!(!o.was_moved(0));
    o.end_loan(loan, pos()).unwrap();
    assert_eq!(o.take_field(0, pos()).unwrap(), object(7, "Cell"));
    assert!(o.was_moved(0));
    assert_eq!(code(o.take_field(0, pos())), "E_MOVE");
}
#[test]
fn partial_move_does_not_invalidate_disjoint_unmoved_field() {
    let mut o = owner(BindingMode::Let);
    o.take_field(0, pos()).unwrap();
    assert!(o.borrow_field(2, LoanKind::Shared, 1, pos()).is_ok());
    assert!(o.read_copy(1, pos()).is_ok());
    assert_eq!(code(o.borrow_root(LoanKind::Shared, 2, pos())), "E_MOVE");
    let ty = o.shape().clone();
    assert_eq!(code(o.rebind_root(&ty, pos())), "E_BORROW"); // live field 2 loan
    o.end_scope(1);
    assert_eq!(code(o.rebind_root(&ty, pos())), "E_MOVE"); // cleanup plan missing
}
#[test]
fn object_copy_actor_send_and_loan_escape_fail_closed() {
    let mut o = owner(BindingMode::Let);
    assert_eq!(code(o.copy_field(0, pos())), "E_UNSUPPORTED");
    assert_eq!(code(o.send_to_actor(0, pos())), "E_UNSUPPORTED");
    let id = o.borrow_field(0, LoanKind::Shared, 1, pos()).unwrap();
    assert_eq!(code(o.escape_loan(id, pos())), "E_BORROW");
    assert!(o.copy_field(1, pos()).is_ok());
}
#[test]
fn per_element_const_uses_literal_provenance_not_root_val_mode() {
    let ty = shape(vec![scalar(Scalar::Int)]);
    let mut o = TupleOwner::new(ty, BindingMode::Val, true, pos()).unwrap();
    let p = o.project(&[binding("answer", BindingMode::Const, false)], 1, pos()).unwrap();
    assert_eq!(p[0].operation, ProjectionOp::CopyValue);
    let ty = shape(vec![scalar(Scalar::Int)]);
    let mut runtime = TupleOwner::new(ty, BindingMode::Let, false, pos()).unwrap();
    assert_eq!(code(runtime.project(&[binding("x", BindingMode::Const, false)], 1, pos())), "E_CONST");
}
#[test]
fn object_element_annotation_uses_resolved_nominal_id_contract() {
    let mut o = owner(BindingMode::Let);
    let mut m = binding("x", BindingMode::Let, false);
    m.annotation = Some("Wrong".into());
    assert_eq!(code(o.project(&[m, discarded(), discarded()], 1, pos())), "E_TYPE");
    assert_eq!(o.active_loans(), 0);
}
#[test]
fn failed_later_element_rolls_back_all_new_loans() {
    let mut o = owner(BindingMode::Let);
    let mut invalid = binding("other", BindingMode::Const, false);
    invalid.annotation = Some("Other".into());
    assert_eq!(code(o.project(&[
        binding("cell", BindingMode::Let, true),
        discarded(),
        invalid,
    ], 1, pos())), "E_CONST");
    assert_eq!(o.active_loans(), 0);
    assert!(o.borrow_field(0, LoanKind::Exclusive, 2, pos()).is_ok());
}
#[test]
fn readonly_binding_and_mutable_referent_are_distinct() {
    let mut o = owner(BindingMode::Let);
    let p = o.project(&[
        binding("cell", BindingMode::Val, true),
        discarded(), discarded(),
    ], 1, pos()).unwrap();
    assert_eq!(p[0].binding, BindingMode::Val);
    assert!(matches!(p[0].operation, ProjectionOp::Borrow {kind: LoanKind::Exclusive, ..}));
}
#[test]
fn too_wide_and_unknown_type_fail_before_planning() {
    let too_wide = vec![scalar(Scalar::Int); 17];
    assert_eq!(code(TupleType::new(too_wide, pos())), "E_SOURCE_LIMIT");
    assert_eq!(code(TupleType::new(vec![object(1, "")], pos())), "E_UNKNOWN_TYPE");
}
#[test]
fn wildcard_does_not_create_a_reference_or_owner() {
    let mut o = owner(BindingMode::Let);
    let p = o.project(&[discarded(), discarded(), discarded()], 1, pos()).unwrap();
    assert!(p.is_empty());
    assert_eq!(o.active_loans(), 0);
}
#[test]
fn scope_end_cannot_release_another_scope_loan() {
    let mut o = owner(BindingMode::Let);
    o.borrow_field(0, LoanKind::Shared, 1, pos()).unwrap();
    o.borrow_field(2, LoanKind::Exclusive, 2, pos()).unwrap();
    o.end_scope(1);
    assert_eq!(o.active_loans(), 1);
    assert!(o.borrow_field(0, LoanKind::Exclusive, 3, pos()).is_ok());
    assert_eq!(code(o.borrow_field(2, LoanKind::Shared, 3, pos())), "E_BORROW");
}

#[test]
fn duplicate_field_names_never_leak_half_acquired_loans() {
    let mut o = owner(BindingMode::Let);
    let failed = o.project(&[
        binding("same", BindingMode::Let, false),
        discarded(),
        binding("same", BindingMode::Let, false),
    ], 9, pos());
    assert_eq!(code(failed), "E_DUPLICATE");
    assert_eq!(o.active_loans(), 0);
}
#[test]
fn rebinding_invalidates_prior_const_provenance_even_with_same_shape() {
    let ty = shape(vec![scalar(Scalar::Int)]);
    let mut o = TupleOwner::new(ty.clone(), BindingMode::Let, true, pos()).unwrap();
    assert!(o.project(&[binding("x", BindingMode::Const, false)], 1, pos()).is_ok());
    o.rebind_root(&ty, pos()).unwrap();
    assert_eq!(code(o.project(&[binding("y", BindingMode::Const, false)], 2, pos())), "E_CONST");
}
#[test]
fn const_root_cannot_acquire_mutable_whole_tuple_loan() {
    let ty = shape(vec![scalar(Scalar::Int)]);
    let mut o = TupleOwner::new(ty, BindingMode::Const, true, pos()).unwrap();
    assert_eq!(code(o.borrow_root(LoanKind::Exclusive, 1, pos())), "E_CONST_MUT_TARGET");
    assert!(o.borrow_root(LoanKind::Shared, 1, pos()).is_ok());
}
