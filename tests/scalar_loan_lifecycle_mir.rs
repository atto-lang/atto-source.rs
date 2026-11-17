//! Executable compiler-origin scalar loan lifecycle facts, not user-authored
//! Polonius grants. The closed diagnostic verifier cannot authorize codegen.
use atto_compiler::{
    check::{self, typed_mir_census::{audit_scalar_loan_lifecycle, ScalarLoanEventKind},
        TStmt, TExpr, TE},
    ast::Type,
    lexer, parser::Parser,
    compile,
};
fn typed(source: &str) -> check::TProgram {
    let ast = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    check::check(&ast).unwrap()
}
fn shared_source() -> &'static str {
    "pub fnc main(): int { let number = 40; val reader = borrow! number; val snapshot = copy! reader; number = 42; return number; }"
}
fn exclusive_source() -> &'static str {
    "pub fnc main(): int { let number = 40; val writer = borrow! mut number; writer = 42; val snapshot = copy! writer; return snapshot; }"
}
#[test]
fn real_checked_scalar_reader_issues_uses_and_ends_before_owner_mutation() {
    let program = typed(shared_source());
    let events = audit_scalar_loan_lifecycle(&program).unwrap();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0].kind, ScalarLoanEventKind::Issue { exclusive: false, .. }));
    assert!(matches!(events[1].kind, ScalarLoanEventKind::Use { write: false, .. }));
    assert!(matches!(events[2].kind, ScalarLoanEventKind::End { .. }));
    assert!(events[0].instruction_ordinal < events[1].instruction_ordinal);
    assert!(events[1].instruction_ordinal < events[2].instruction_ordinal);
    assert_eq!(events[0].loan_id, events[1].loan_id);
    assert_eq!(events[1].loan_id, events[2].loan_id);
    assert_eq!(compile(shared_source()).unwrap()[..8], [0,97,115,109,1,0,0,0]);
}
#[test]
fn exclusive_write_is_checked_as_a_distinct_borrow_use() {
    let program = typed(exclusive_source());
    let events = audit_scalar_loan_lifecycle(&program).unwrap();
    assert_eq!(events.len(), 4);
    assert!(matches!(events[0].kind, ScalarLoanEventKind::Issue { exclusive: true, .. }));
    assert!(matches!(events[1].kind, ScalarLoanEventKind::Use { write: true, .. }));
    assert!(matches!(events[2].kind, ScalarLoanEventKind::Use { write: false, .. }));
    assert!(matches!(events[3].kind, ScalarLoanEventKind::End { .. }));
    assert!(compile(exclusive_source()).is_ok());
}
#[test]
fn drop_alias_use_marker_rejects_unproven_early_loan_end() {
    let mut p = typed(shared_source());
    p.funcs[0].body.retain(|s| !matches!(s, TStmt::ScalarLoanUse { .. }));
    let e = audit_scalar_loan_lifecycle(&p).unwrap_err();
    assert_eq!(e.code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}
#[test]
fn moving_use_after_end_is_rejected() {
    let mut p = typed(shared_source());
    let func = &mut p.funcs[0];
    let use_pos = func.body.iter().position(|s| matches!(s, TStmt::ScalarLoanUse { .. })).unwrap();
    let use_event = func.body.remove(use_pos);
    let end = func.body.iter().position(|s| matches!(s, TStmt::ScalarLoanEnd { .. })).unwrap();
    func.body.insert(end + 1, use_event);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}
#[test]
fn conflicting_write_inserted_before_proven_end_fails_closed() {
    let mut p = typed(shared_source());
    let func = &mut p.funcs[0];
    let insert_at = func.body.iter().position(|s| matches!(s, TStmt::ScalarLoanEnd { .. })).unwrap();
    func.body.insert(insert_at, TStmt::Set(0, TE { e: TExpr::I64(41), ty: Type::Int }));
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}
#[test]
fn duplicated_loan_id_and_mismatched_owner_are_rejected() {
    let mut p = typed(shared_source());
    let issue = p.funcs[0].body.iter().find(|s| matches!(s, TStmt::ScalarBorrowIssue { .. })).unwrap().clone();
    p.funcs[0].body.insert(2, issue);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
    let mut p = typed(shared_source());
    let item = p.funcs[0].body.iter_mut().find(|s| matches!(s, TStmt::ScalarLoanUse { .. })).unwrap();
    if let TStmt::ScalarLoanUse { owner_slot, .. } = item { *owner_slot = 999; }
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}
#[test]
fn incomplete_lifetime_at_return_requires_explicit_cleanup_proof() {
    let src = "pub fnc main(): int { let number = 40; val reader = borrow! number; return number; }";
    let p = typed(src);
    assert!(compile(src).is_ok());
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
}
#[test]
fn effectful_calls_and_conditional_loans_are_not_authorized_by_flat_census() {
    let with_call = include_str!("fixtures/pass/ownership-borrow-nll-last-use.atto");
    let p = typed(with_call);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
    let branch = include_str!("fixtures/pass/ownership-borrow-branch-release.atto");
    assert_eq!(audit_scalar_loan_lifecycle(&typed(branch)).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
}
#[test]
fn actual_unsafe_program_is_rejected_before_diagnostic_manifest() {
    let unsafe_source = include_str!("fixtures/fail/ownership-borrow-nll-live-after-mutation.atto");
    assert_eq!(compile(unsafe_source).unwrap_err().code, "E_BORROW");
}
