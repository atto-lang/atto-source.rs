//! Checked typed-MIR conditional CFG and independently recomputed MAY-use
//! proof. The oracle never authorizes production codegen.
use atto_compiler::{
    ast::Type,
    check::{
        self,
        loan_cfg_oracle_v1::{audit_checked_program, Kind},
        TExpr, TStmt, TE,
    },
    compile, lexer,
    parser::Parser,
};
fn typed(source: &str) -> check::TProgram {
    let tokens = lexer::lex(source).unwrap();
    let ast = Parser::new(tokens).parse_program().unwrap();
    check::check(&ast).unwrap()
}
fn branches() -> &'static str {
    "pub fnc main(): int {
      let number = 40;
      val choose = true;
      if choose then
          val reader = borrow! number;
          val snapshot = copy! reader;
      else
          val alternative = borrow! number;
          val duplicate = copy! alternative;
      fi
      number = 42;
      return number;
    }"
}
fn nested_block() -> &'static str {
    "pub fnc main(): int {
      let number = 40;
      block {
        val reader = borrow! number;
        val saved = copy! reader;
      }
      number = 42;
      return number;
    }"
}
#[test]
fn two_real_compiled_branch_local_loans_close_before_common_owner_write() {
    let source = branches();
    assert!(compile(source).is_ok());
    let report = audit_checked_program(&typed(source)).unwrap();
    let main = report.functions.iter().find(|f| f.name == "main").unwrap();
    let issued = main.points.iter().filter(|p| matches!(p.kind, Kind::Issue { .. })).count();
    let ends = main
        .points
        .iter()
        .filter(|p| matches!(p.kind, Kind::End { lexical: true, .. }))
        .count();
    let branches = main.points.iter().filter(|p| matches!(p.kind, Kind::Condition)).count();
    assert_eq!((issued, ends, branches), (2, 2, 1));
    assert_eq!(
        main.points
            .iter()
            .find(|p| matches!(p.kind, Kind::Condition))
            .unwrap()
            .successors
            .len(),
        2
    );
}
#[test]
fn nested_lexical_block_emits_checked_scope_close_and_is_safe() {
    assert!(compile(nested_block()).is_ok());
    let report = audit_checked_program(&typed(nested_block())).unwrap();
    assert_eq!(
        report.functions[0]
            .points
            .iter()
            .filter(|p| matches!(p.kind, Kind::End { lexical: true, .. }))
            .count(),
        1
    );
}
#[test]
fn nll_last_use_is_recomputed_without_trusting_end_marker() {
    let source = "pub fnc main(): int {
      let number = 40;
      val reader = borrow! number;
      val first = copy! reader;
      val second = copy! reader;
      number = 42;
      return number;
    }";
    let mut p = typed(source);
    assert!(audit_checked_program(&p).is_ok());
    let body = &mut p.funcs[0].body;
    let end = body.iter().position(|s| matches!(s, TStmt::ScalarLoanEnd { .. })).unwrap();
    let marker = body.remove(end);
    let last_use = body
        .iter()
        .rposition(|s| matches!(s, TStmt::ScalarLoanUse { write: false, .. }))
        .unwrap();
    body.insert(last_use, marker);
    let error = audit_checked_program(&p).unwrap_err();
    assert_eq!(error.code, "E_LOAN_CFG_INCOMPLETE");
}
#[test]
fn inserted_future_typed_alias_use_after_end_is_rejected() {
    let mut p = typed(nested_block());
    // Forge an extra guest alias load after lexical scope-close. This
    // checks liveness independently of the original source AST scanner.
    let issue = p.funcs[0]
        .body
        .iter()
        .find_map(|s| {
            if let TStmt::Block(b) = s {
                b.iter().find_map(|s| {
                    if let TStmt::ScalarBorrowIssue { loan_id, owner_slot, .. } = s {
                        Some((*loan_id, *owner_slot))
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
        .unwrap();
    let body = &mut p.funcs[0].body;
    let at = body.iter().position(|s| matches!(s, TStmt::Block(..))).unwrap();
    body.insert(
        at + 1,
        TStmt::ScalarLoanUse {
            loan_id: issue.0,
            owner_slot: issue.1,
            write: false,
        },
    );
    body.insert(
        at + 2,
        TStmt::Expr(TE {
            e: TExpr::LoanRead {
                loan_id: issue.0,
                owner_slot: issue.1,
            },
            ty: Type::Int,
        }),
    );
    assert_eq!(audit_checked_program(&p).unwrap_err().code, "E_LOAN_CFG_INCOMPLETE");
}
#[test]
fn missing_scope_end_on_one_branch_must_not_be_implicitly_assumed() {
    let mut p = typed(branches());
    if let Some(TStmt::If(arms, _)) = p.funcs[0].body.iter_mut().find(|s| matches!(s, TStmt::If(..))) {
        arms[0].1.retain(|s| !matches!(s, TStmt::ScalarLoanScopeEnd { .. }));
    }
    assert!(audit_checked_program(&p).is_err());
}
#[test]
fn repeated_loan_id_across_distinct_branches_is_not_a_valid_renaming() {
    let mut p = typed(branches());
    if let Some(TStmt::If(arms, Some(other))) = p.funcs[0].body.iter_mut().find(|s| matches!(s, TStmt::If(..))) {
        let loan_id = arms[0]
            .1
            .iter()
            .find_map(|s| {
                if let TStmt::ScalarBorrowIssue { loan_id, .. } = s {
                    Some(*loan_id)
                } else {
                    None
                }
            })
            .unwrap();
        for stmt in other {
            match stmt {
                TStmt::ScalarBorrowIssue { loan_id: id, .. }
                | TStmt::ScalarLoanUse { loan_id: id, .. }
                | TStmt::ScalarLoanScopeEnd { loan_id: id, .. } => *id = loan_id,
                _ => {}
            }
        }
    }
    assert_eq!(audit_checked_program(&p).unwrap_err().code, "E_LOAN_CFG_INCOMPLETE");
}
#[test]
fn direct_owner_read_under_branch_local_exclusive_borrow_rejects() {
    let source = "pub fnc main(): int {
      let number = 40;
      if true then
        val writer = borrow! mut number;
        writer = 42;
      fi
      return number;
    }";
    let mut p = typed(source);
    let body = &mut p.funcs[0].body;
    if let Some(TStmt::If(arms, _)) = body.iter_mut().find(|s| matches!(s, TStmt::If(..))) {
        let index = arms[0].1.iter().position(|s| matches!(s, TStmt::ScalarBorrowIssue { .. })).unwrap();
        arms[0].1.insert(
            index + 1,
            TStmt::Expr(TE {
                e: TExpr::Local(0),
                ty: Type::Int,
            }),
        );
    }
    assert_eq!(audit_checked_program(&p).unwrap_err().code, "E_LOAN_CFG_INCOMPLETE");
}
#[test]
fn unknown_loop_or_abrupt_exits_remain_fail_closed() {
    let mut p = typed(nested_block());
    p.funcs[0].body.insert(
        0,
        TStmt::Loop {
            cond: Some(TE {
                e: TExpr::Bool(false),
                ty: Type::Bool,
            }),
            body: vec![],
            update: vec![],
        },
    );
    assert_eq!(audit_checked_program(&p).unwrap_err().code, "E_LOAN_CFG_UNSUPPORTED");
}
#[test]
fn typed_cfg_budget_does_not_allow_unbounded_point_amplification() {
    let mut p = typed(nested_block());
    let mut repeated = Vec::new();
    for _ in 0..1100 {
        repeated.push(TStmt::Expr(TE {
            e: TExpr::I64(1),
            ty: Type::Int,
        }));
    }
    repeated.extend(p.funcs[0].body.clone());
    p.funcs[0].body = repeated;
    assert_eq!(audit_checked_program(&p).unwrap_err().code, "E_LOAN_CFG_BUDGET");
}
#[test]
fn input_to_cfg_is_deterministic() {
    let p = typed(branches());
    let a = audit_checked_program(&p).unwrap();
    let b = audit_checked_program(&p).unwrap();
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}
