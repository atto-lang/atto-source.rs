//! Executed compiler fact-model tests: use real parser/typechecker and same
//! checked typed IR as the native/wasm guest emitter, no handwritten loan
//! facts for positive cases. This model stays non-authoritative for codegen.
use atto_compiler::{
    check::{
        self,
        scalar_loan_census::{self, Event},
        TStmt,
    },
    compile, compile_with_scalar_loan_census, compile_with_typed_scalar_census, lexer,
    parser::Parser,
};

fn typed(src: &str) -> check::TProgram {
    let program = Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    check::check(&program).unwrap()
}
fn loan_events(src: &str) -> Vec<Event> {
    let (bytes, audit) = compile_with_scalar_loan_census(src).unwrap();
    assert_eq!(bytes, compile(src).unwrap(), "diagnostics cannot alter emitted guest bytes");
    audit
        .functions
        .iter()
        .find(|f| f.name == "main")
        .unwrap()
        .points
        .iter()
        .map(|p| p.event.clone())
        .collect()
}
fn pos(events: &[Event], predicate: impl Fn(&Event) -> bool) -> usize {
    events.iter().position(predicate).expect("expected compiler-issued event")
}
#[test]
fn real_scalar_last_use_ends_before_owner_write_and_matches_guest_bytes() {
    let source = include_str!("fixtures/pass/ownership-borrow-nll-pure-suffix.atto");
    let events = loan_events(source);
    let issue = pos(&events, |e| {
        matches!(
            e,
            Event::Issue {
                loan_id: 1,
                owner_slot: 0,
                exclusive: false
            }
        )
    });
    let used = pos(&events, |e| {
        matches!(
            e,
            Event::Use {
                loan_id: 1,
                owner_slot: 0,
                write: false
            }
        )
    });
    let ended = pos(&events, |e| {
        matches!(
            e,
            Event::End {
                loan_id: 1,
                owner_slot: 0,
                scope_exit: false
            }
        )
    });
    let owner_writes: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Write { owner_slot: 0, .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(owner_writes.len(), 2);
    assert!(owner_writes[0] < issue && issue < used && used < ended && ended < owner_writes[1]);
    assert!(matches!(events.last(), Some(Event::Return)));
    assert_eq!(
        compile_with_typed_scalar_census(source).unwrap_err().code,
        "E_TYPED_MIR_CENSUS_UNSUPPORTED"
    );
}
#[test]
fn exclusive_writer_marks_permission_then_owner_set_then_nll_end() {
    let source = include_str!("fixtures/pass/ownership-borrow-nll-writer-release.atto");
    let events = loan_events(source);
    let issue = pos(&events, |e| {
        matches!(
            e,
            Event::Issue {
                loan_id: 1,
                exclusive: true,
                ..
            }
        )
    });
    let write = pos(&events, |e| {
        matches!(
            e,
            Event::Use {
                loan_id: 1,
                write: true,
                ..
            }
        )
    });
    let end = pos(&events, |e| {
        matches!(
            e,
            Event::End {
                loan_id: 1,
                scope_exit: false,
                ..
            }
        )
    });
    let writes: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Event::Write { owner_slot: 0, .. }))
        .map(|(i, _)| i)
        .collect();
    assert!(issue < write && write < writes[1] && writes[1] < end && end < writes[2]);
}
#[test]
fn shared_to_exclusive_reborrow_transition_requires_prior_end() {
    let source = include_str!("fixtures/pass/ownership-borrow-nll-exclusive-transition.atto");
    let events = loan_events(source);
    let end_shared = pos(&events, |e| matches!(e, Event::End { loan_id: 1, .. }));
    let issue_exclusive = pos(&events, |e| {
        matches!(
            e,
            Event::Issue {
                loan_id: 2,
                exclusive: true,
                ..
            }
        )
    });
    assert!(end_shared < issue_exclusive);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Use {
            loan_id: 2,
            write: true,
            ..
        }
    )));
}
#[test]
fn lexical_scope_fallthrough_closes_loan_and_second_scope_gets_fresh_id() {
    let src = r#"
pub fnc main(): int {
  let number = 40;
  block {
    val reader = borrow! number;
    val snapshot = copy! reader;
  }
  block {
    val reader = borrow! number;
    val snapshot = copy! reader;
  }
  number = 42;
  return number;
}
"#;
    let events = loan_events(src);
    let first = pos(&events, |e| matches!(e, Event::Issue { loan_id: 1, .. }));
    let first_end = pos(&events, |e| {
        matches!(
            e,
            Event::End {
                loan_id: 1,
                scope_exit: true,
                ..
            }
        )
    });
    let second = pos(&events, |e| matches!(e, Event::Issue { loan_id: 2, .. }));
    let second_end = pos(&events, |e| {
        matches!(
            e,
            Event::End {
                loan_id: 2,
                scope_exit: true,
                ..
            }
        )
    });
    assert!(first < first_end && first_end < second && second < second_end);
}
#[test]
fn emitted_return_with_unescaped_last_use_closes_live_loan() {
    let src = "pub fnc main(): int { let x = 42; val r = borrow! x; return copy! r; }";
    let events = loan_events(src);
    let i = pos(&events, |e| matches!(e, Event::Issue { loan_id: 1, .. }));
    let u = pos(&events, |e| {
        matches!(
            e,
            Event::Use {
                loan_id: 1,
                write: false,
                ..
            }
        )
    });
    assert!(i < u);
    assert!(matches!(events.last(), Some(Event::Return)));
}
#[test]
fn source_rejection_precedes_any_diagnostic_lifetime_approval() {
    for source in [
        include_str!("fixtures/fail/ownership-borrow-nll-live-after-mutation.atto"),
        include_str!("fixtures/fail/ownership-borrow-nll-still-live-at-exclusive-reborrow.atto"),
    ] {
        assert_eq!(compile(source).unwrap_err().code, "E_BORROW");
        assert_eq!(compile_with_scalar_loan_census(source).unwrap_err().code, "E_BORROW");
    }
}
#[test]
fn unsupported_effect_and_branch_are_diagnostic_fail_closed_only() {
    let print = include_str!("fixtures/pass/ownership-borrow-nll-last-use.atto");
    assert!(compile(print).is_ok());
    assert_eq!(
        compile_with_scalar_loan_census(print).unwrap_err().code,
        "E_LOAN_CENSUS_UNSUPPORTED"
    );
    let branch = include_str!("fixtures/pass/ownership-borrow-branch-release.atto");
    assert!(compile(branch).is_ok());
    assert_eq!(
        compile_with_scalar_loan_census(branch).unwrap_err().code,
        "E_LOAN_CENSUS_UNSUPPORTED"
    );
}
#[test]
fn tampered_typed_use_after_loan_end_is_diagnostic_rejection() {
    let src = include_str!("fixtures/pass/ownership-borrow-nll-pure-suffix.atto");
    let mut program = typed(src);
    let body = &mut program.funcs[0].body;
    let at = body
        .iter()
        .position(|x| matches!(x, TStmt::ScalarBorrowEnd { loan_id: 1, .. }))
        .unwrap();
    body.insert(
        at + 1,
        TStmt::ScalarBorrowUse {
            loan_id: 1,
            owner_slot: 0,
            write: false,
        },
    );
    assert_eq!(
        scalar_loan_census::from_checked_program(&program).unwrap_err().code,
        "E_LOAN_CENSUS_INVALID"
    );
}
#[test]
fn tampered_typed_early_loan_end_does_not_grant_owner_write() {
    let src = include_str!("fixtures/pass/ownership-borrow-nll-pure-suffix.atto");
    let mut program = typed(src);
    program.funcs[0]
        .body
        .retain(|stmt| !matches!(stmt, TStmt::ScalarBorrowEnd { loan_id: 1, .. }));
    assert_eq!(
        scalar_loan_census::from_checked_program(&program).unwrap_err().code,
        "E_LOAN_CENSUS_INVALID"
    );
}
#[test]
fn duplicate_loan_id_in_typed_program_is_rejected() {
    let src = include_str!("fixtures/pass/ownership-borrow-nll-exclusive-transition.atto");
    let mut program = typed(src);
    for item in &mut program.funcs[0].body {
        if let TStmt::ScalarBorrowIssue { loan_id, alias, .. } = item {
            if alias == "writer" {
                *loan_id = 1;
            }
        }
    }
    assert_eq!(
        scalar_loan_census::from_checked_program(&program).unwrap_err().code,
        "E_LOAN_CENSUS_INVALID"
    );
}
#[test]
fn fake_shared_write_through_cannot_pass_model() {
    let src = include_str!("fixtures/pass/ownership-borrow-nll-pure-suffix.atto");
    let mut program = typed(src);
    for item in &mut program.funcs[0].body {
        if let TStmt::ScalarBorrowUse { write, .. } = item {
            *write = true;
        }
    }
    assert_eq!(
        scalar_loan_census::from_checked_program(&program).unwrap_err().code,
        "E_LOAN_CENSUS_INVALID"
    );
}
#[test]
fn two_checks_produce_identical_facts_and_guest_bytes() {
    let source = include_str!("fixtures/pass/ownership-borrow-nll-pure-suffix.atto");
    let (bytes_a, report_a) = compile_with_scalar_loan_census(source).unwrap();
    let (bytes_b, report_b) = compile_with_scalar_loan_census(source).unwrap();
    assert_eq!(bytes_a, bytes_b);
    assert_eq!(report_a, report_b);
}
