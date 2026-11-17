//! Executable compiler-origin scalar loan lifecycle facts, not user-authored
//! Polonius grants. The closed diagnostic verifier cannot authorize codegen.
use atto_compiler::{
    ast::Type,
    check::{
        self,
        typed_mir_census::{audit_scalar_loan_lifecycle, ScalarLoanEventKind},
        TExpr, TStmt, TE,
    },
    compile, lexer,
    parser::Parser,
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
    assert_eq!(compile(shared_source()).unwrap()[..8], [0, 97, 115, 109, 1, 0, 0, 0]);
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
    func.body.insert(
        insert_at,
        TStmt::Set(
            0,
            TE {
                e: TExpr::I64(41),
                ty: Type::Int,
            },
        ),
    );
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}
#[test]
fn duplicated_loan_id_and_mismatched_owner_are_rejected() {
    let mut p = typed(shared_source());
    let issue = p.funcs[0]
        .body
        .iter()
        .find(|s| matches!(s, TStmt::ScalarBorrowIssue { .. }))
        .unwrap()
        .clone();
    p.funcs[0].body.insert(2, issue);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
    let mut p = typed(shared_source());
    let item = p.funcs[0]
        .body
        .iter_mut()
        .find(|s| matches!(s, TStmt::ScalarLoanUse { .. }))
        .unwrap();
    if let TStmt::ScalarLoanUse { owner_slot, .. } = item {
        *owner_slot = 999;
    }
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
    assert_eq!(
        audit_scalar_loan_lifecycle(&typed(branch)).unwrap_err().code,
        "E_TYPED_MIR_CENSUS_UNSUPPORTED"
    );
}
#[test]
fn actual_unsafe_program_is_rejected_before_diagnostic_manifest() {
    let unsafe_source = include_str!("fixtures/fail/ownership-borrow-nll-live-after-mutation.atto");
    assert_eq!(compile(unsafe_source).unwrap_err().code, "E_BORROW");
}


#[test]
fn alias_reads_in_checked_expressions_retain_unique_loan_provenance() {
    let p = typed(shared_source());
    let (id, owner_slot) = p.funcs[0].body.iter().find_map(|s| {
        if let TStmt::ScalarBorrowIssue { loan_id, owner_slot, .. } = s {
            Some((*loan_id, *owner_slot))
        } else {
            None
        }
    }).expect("compiled borrow issue");
    assert!(p.funcs[0].body.iter().any(|s| matches!(
        s,
        TStmt::Set(_, TE {
            e: TExpr::LoanRead { loan_id, owner_slot: slot },
            ..
        }) if *loan_id == id && *slot == owner_slot
    )));
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap().len(), 3);
}

#[test]
fn missing_one_of_two_alias_use_markers_is_rejected_even_if_prior_use_exists() {
    let source = "pub fnc main(): int { let number = 40; val reader = borrow! number; val first = copy! reader; val second = copy! reader; number = 42; return number; }";
    let mut p = typed(source);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap().len(), 4);
    let use_indices: Vec<_> = p.funcs[0].body.iter().enumerate()
        .filter_map(|(i,s)| matches!(s, TStmt::ScalarLoanUse { write: false, .. }).then_some(i))
        .collect();
    assert_eq!(use_indices.len(), 2);
    p.funcs[0].body.remove(use_indices[1]);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}

#[test]
fn fabricated_loan_use_marker_with_plain_direct_load_is_rejected() {
    let mut p = typed(shared_source());
    let stmt = p.funcs[0].body.iter_mut().find(|s| matches!(
        s, TStmt::Set(_, TE { e: TExpr::LoanRead { .. }, .. })
    )).expect("loan alias value use");
    if let TStmt::Set(_, TE { e, .. }) = stmt {
        let TExpr::LoanRead { owner_slot, .. } = e else { unreachable!() };
        *e = TExpr::Local(*owner_slot);
    }
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}

#[test]
fn duplicate_loan_read_marker_without_second_guest_load_is_rejected() {
    let mut p = typed(shared_source());
    let first_use = p.funcs[0].body.iter().find(|s| matches!(
        s, TStmt::ScalarLoanUse { write: false, .. }
    )).unwrap().clone();
    let pos = p.funcs[0].body.iter().position(|s| matches!(
        s, TStmt::ScalarLoanUse { write: false, .. }
    )).unwrap();
    p.funcs[0].body.insert(pos, first_use);
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}

#[test]
fn direct_owner_read_under_exclusive_loan_cannot_hide_as_plain_typed_local() {
    let mut p = typed(exclusive_source());
    let issue = p.funcs[0].body.iter().position(|s| matches!(
        s, TStmt::ScalarBorrowIssue { exclusive: true, .. }
    )).unwrap();
    p.funcs[0].body.insert(issue + 1, TStmt::Expr(TE {
        e: TExpr::Local(0),
        ty: Type::Int,
    }));
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}

#[test]
fn wrong_alias_loan_id_inside_real_guest_load_is_rejected() {
    let mut p = typed(shared_source());
    for stmt in &mut p.funcs[0].body {
        if let TStmt::Set(_, TE { e: TExpr::LoanRead { loan_id, .. }, .. }) = stmt {
            *loan_id = 999;
            break;
        }
    }
    assert_eq!(audit_scalar_loan_lifecycle(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
}

#[test]
fn possible_trap_with_live_scalar_loan_requires_exceptional_cfg() {
    let source = "pub fnc main(): int { let number = 40; val reader = borrow! number; val total = copy! reader + 2; number = 42; return number; }";
    let program = typed(source);
    assert!(compile(source).is_ok());
    assert_eq!(audit_scalar_loan_lifecycle(&program).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
}

#[test]
fn unsupported_typed_borrow_read_cannot_launder_into_pure_scalar_census() {
    let mut p = typed(shared_source());
    p.funcs[0].body.retain(|s| !matches!(
        s, TStmt::ScalarBorrowIssue { .. } | TStmt::ScalarLoanUse { .. } | TStmt::ScalarLoanEnd { .. }
    ));
    assert_eq!(
        atto_compiler::check::typed_mir_census::from_checked_program(&p).unwrap_err().code,
        "E_TYPED_MIR_CENSUS_UNSUPPORTED"
    );
}
