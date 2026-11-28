//! L3 borrow-conformance exporter (`borrow_export`, profile structured-cfg-v1).
//!
//! These golden tests pin the exact canonical ownership events the checker
//! records, so that a change in the checker's ownership decisions or in the
//! mapping shows up here before it reaches the master cross-check
//! (`atto-borrow-conform cross-check` in atto-lang/atto-borrow-checker.rs).
use atto_compiler::borrow_export::{self, Record};
use std::process::Command;

const CLASS: &str = "define class C as pub let int x = 1; pub get(): int { return self.x; } end\n";

fn export(body: &str) -> Vec<Record> {
    borrow_export::begin();
    let _ = atto_compiler::compile(&format!("{CLASS}{body}"));
    borrow_export::finish()
}

fn main_record(body: &str) -> Record {
    export(body)
        .into_iter()
        .find(|r| r.function == "main")
        .expect("main must be recorded")
}

fn ops(record: &Record) -> Vec<String> {
    match &record.mir {
        Ok(events) => events.iter().map(|(e, _)| e.short()).collect(),
        Err(reason) => panic!("expected an exported graph, got mirUnavailable: {reason}"),
    }
}

fn code(record: &Record) -> Option<&str> {
    record.local.as_ref().map(|(c, _)| c.as_str())
}

#[test]
fn disabled_by_default() {
    let _ = atto_compiler::compile(&format!("{CLASS}pub fnc main(): int {{ return 1; }}"));
    assert!(borrow_export::finish().is_empty());
    assert!(!borrow_export::enabled());
}

#[test]
fn take_retargets_into_the_binding_and_rejects_use_after_move_as_a_prefix() {
    let r = main_record("pub fnc main(): int { val a = new C(); val b = take! a; return a.x; }");
    assert_eq!(code(&r), Some("E_MOVED"));
    assert!(r.prefix_at_error);
    assert_eq!(ops(&r), ["Init(1)", "Read(1)", "Move(1->2)", "Read(1)", "Return"]);
}

#[test]
fn shared_loan_allows_owner_reads_and_ends_before_return() {
    let r = main_record("pub fnc main(): int { val c = new C(); val v = borrow! c; return v.x + c.x; }");
    assert_eq!(code(&r), None);
    assert!(!r.prefix_at_error);
    assert_eq!(
        ops(&r),
        ["Init(1)", "Borrow(1,2,shared)", "Read(1)", "Read(1)", "EndBorrow(2)", "Return"]
    );
}

#[test]
fn access_through_a_mutable_binder_is_write_through() {
    let r = main_record("pub fnc main(): int { let c = new C(); val w = borrow! mut c; return w.x; }");
    assert_eq!(code(&r), None);
    assert_eq!(
        ops(&r),
        ["Init(1)", "Borrow(1,2,mut)", "WriteThrough(1,2)", "EndBorrow(2)", "Return"]
    );
}

#[test]
fn owner_read_during_mutable_loan_rejects_at_the_read() {
    let r = main_record("pub fnc main(): int { let c = new C(); val w = borrow! mut c; return c.x + w.x; }");
    assert_eq!(code(&r), Some("E_BORROW"));
    assert_eq!(ops(&r), ["Init(1)", "Borrow(1,2,mut)", "Read(1)", "EndBorrow(2)", "Return"]);
}

#[test]
fn block_scope_end_closes_the_loan_before_a_later_move() {
    let r = main_record(
        "pub fnc main(): int { let c = new C(); block { val v = borrow! c; val n = v.x; } val m = take! c; return m.x; }",
    );
    assert_eq!(code(&r), None);
    assert_eq!(
        ops(&r),
        [
            "Init(1)", "Borrow(1,2,shared)", "Read(1)", "Init(3,copy)", "EndBorrow(2)",
            "Read(1)", "Move(1->4)", "Read(4)", "Return",
        ]
    );
}

#[test]
fn return_inside_a_block_closes_live_loans_and_records_nothing_after() {
    let r = main_record("pub fnc main(): int { let c = new C(); block { val v = borrow! c; return v.x; } }");
    assert_eq!(code(&r), None);
    assert_eq!(ops(&r), ["Init(1)", "Borrow(1,2,shared)", "Read(1)", "EndBorrow(2)", "Return"]);
}

#[test]
fn passing_a_class_local_moves_it_on_current_main() {
    // Faithful to this compiler today: ordinary arguments consume class
    // locals. The spec's implicit-argument-borrow-v1 says they should borrow;
    // L2 (BCV1-003) tracks that divergence, not this exporter.
    let r = main_record(
        "fnc f(C p): int { return p.x; } pub fnc main(): int { val c = new C(); val a = f(c); return c.x; }",
    );
    assert_eq!(code(&r), Some("E_MOVED"));
    assert_eq!(ops(&r), ["Init(1)", "Read(1)", "Move(1->1000001)", "Init(2,copy)", "Read(1)", "Return"]);
}

#[test]
fn dropped_scalar_reads_precede_release() {
    let ok = main_record("pub fnc main(): int { val int x = 5; drop! x; return 42; }");
    assert_eq!(ops(&ok), ["Init(1,copy)", "Read(1)", "Drop(1)", "Return"]);
    let bad = main_record("pub fnc main(): int { val int x = 5; drop! x; return x; }");
    assert_eq!(code(&bad), Some("E_MOVED"));
    assert_eq!(ops(&bad), ["Init(1,copy)", "Read(1)", "Drop(1)", "Read(1)", "Return"]);
}

#[test]
fn parameters_are_initialized_owners() {
    let records = export("fnc f(C p, int n): int { return p.x + n; } pub fnc main(): int { return 1; }");
    let f = records.iter().find(|r| r.function == "f").unwrap();
    assert_eq!(ops(f), ["Init(1)", "Init(2,copy)", "Read(1)", "Read(2)", "Return"]);
}

#[test]
fn shapes_outside_the_profile_fail_closed_with_a_reason() {
    for (body, reason) in [
        ("pub fnc main(): int { val int x = 1; match x over on 1 -> { return 1; } on _ -> { return 2; } end }", "match"),
        ("pub fnc main(): int { val c = new C(); return c.get(); }", "method call"),
        ("pub fnc main(): int { let int x = 1; x = 2; return x; }", "reassignment"),
        ("pub fnc main(): int { let int i = 0; while i < 3 { i = i + 1; } return i; }", "reassignment"),
    ] {
        let r = main_record(body);
        match &r.mir {
            Err(why) => assert!(why.contains(reason), "{body}: {why}"),
            Ok(_) => panic!("{body}: exported a graph outside structured-cfg-v1"),
        }
    }
}

#[test]
fn class_methods_are_recorded_as_unavailable() {
    let records = export("pub fnc main(): int { return 1; }");
    let get = records.iter().find(|r| r.function == "C.get").expect("method body recorded");
    assert!(matches!(&get.mir, Err(why) if why.contains("class")));
}

#[test]
fn non_ownership_rejects_do_not_export_a_partial_graph() {
    // freeze! of a class has no deep-immutability witness on this backend.
    let r = main_record("pub fnc main(): int { val c = new C(); val d = freeze! c; return 1; }");
    assert_eq!(code(&r), Some("E_FREEZE_PROOF"));
    assert!(matches!(&r.mir, Err(why) if why.contains("non-ownership")));
}

// ---------------------------------------------------------------------------
// attoc --borrow-report
// ---------------------------------------------------------------------------

fn attoc(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_attoc"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("attoc must be launchable")
}

fn temp_report(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("attoc-borrow-{}-{name}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn cli_writes_backend_report_lines_and_keeps_exit_codes() {
    let out = temp_report("fail");
    let output = attoc(&[
        "--check", "tests/fixtures/fail/151-borrow-use-after-take.atto",
        "--borrow-report", out.to_str().unwrap(), "--compiler-git-sha", SHA, "--case", "X-1",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", String::from_utf8_lossy(&output.stderr));
    let text = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    assert!(!text.is_empty());
    for line in text.lines() {
        assert!(line.starts_with(r#"{"schema":"atto-borrow-backend-report-v1","backend":"rs""#), "{line}");
        assert!(line.contains(&format!(r#""compilerGitSha":"{SHA}""#)));
        assert!(line.contains(r#""case":"X-1""#));
        assert!(line.contains(r#""mir":{"version":"atto-borrow-mir-v1""#) || line.contains(r#""mirUnavailable":"#));
    }
    assert!(text.contains(r#""verdict":"reject","code":"E_MOVED""#), "{text}");
}

#[test]
fn cli_report_requires_a_compiler_sha_and_rejects_stray_flags() {
    let out = temp_report("nosha");
    let missing = attoc(&["--check", "tests/fixtures/pass/01-answer.atto", "--borrow-report", out.to_str().unwrap()]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(!out.exists());
    let short = attoc(&["--check", "tests/fixtures/pass/01-answer.atto",
        "--borrow-report", out.to_str().unwrap(), "--compiler-git-sha", "abc"]);
    assert_eq!(short.status.code(), Some(2));
    let stray = attoc(&["--check", "tests/fixtures/pass/01-answer.atto", "--case", "X"]);
    assert_eq!(stray.status.code(), Some(2));
    let dangling = attoc(&["--check", "tests/fixtures/pass/01-answer.atto", "--borrow-report"]);
    assert_eq!(dangling.status.code(), Some(2));
}

// ---------------------------------------------------------------------------
// Escaping loans and compiler-limit witnesses (found by the first full
// cross-check of tests/fixtures: each was a mapping defect before).
// ---------------------------------------------------------------------------

#[test]
fn transferring_a_loan_binder_leaves_the_loan_live_at_return() {
    // Not a Move of a nonexistent owner: the loan escapes its lexical scope,
    // which canonical verification reports as E_LOAN_ESCAPE (family E_BORROW).
    let r = main_record("pub fnc main(): int { let c = new C(); val v = borrow! c; val alias = v; return 1; }");
    assert_eq!(code(&r), Some("E_BORROW"));
    assert_eq!(ops(&r), ["Init(1)", "Borrow(1,2,shared)", "Read(1)", "Return"]);
}

#[test]
fn expression_level_borrow_is_born_escaping() {
    let records = export("fnc leak(C owner): C { return borrow! owner; } pub fnc main(): int { return 0; }");
    let leak = records.iter().find(|r| r.function == "leak").unwrap();
    assert_eq!(code(leak), Some("E_BORROW"));
    assert_eq!(ops(leak), ["Init(1)", "Read(1)", "Borrow(1,1000001,shared)", "Return"]);
}

#[test]
fn reborrow_has_no_canonical_op_and_is_not_exported() {
    let r = main_record("pub fnc main(): int { let c = new C(); val a = borrow! c; val b = borrow! a; return 1; }");
    assert!(matches!(&r.mir, Err(why) if why.contains("reborrow")));
}

#[test]
fn compiler_limits_export_a_faithful_witness() {
    // Both are legal in the canonical model, so the master classifies them
    // BACKEND_STRICTER: the graph must show the loan the checker refused.
    let scalar = main_record("pub fnc main(): int { let amount = 42; val w = borrow! mut amount; return 1; }");
    assert_eq!(code(&scalar), Some("E_BORROW"));
    assert_eq!(ops(&scalar), ["Init(1,copy)", "Borrow(1,2,mut)", "EndBorrow(2)", "Return"]);
    let temp = main_record("pub fnc main(): int { val r = borrow! new C(); return 1; }");
    assert_eq!(code(&temp), Some("E_BORROW"));
    assert_eq!(ops(&temp), ["Init(1000001)", "Borrow(1000001,1,shared)", "EndBorrow(1)", "Return"]);
}

#[test]
fn unused_exclusive_loan_ends_before_owner_read_in_export() {
    let r = main_record("pub fnc main(): int { let c = new C(); val w = borrow! mut c; return c.x; }");
    assert_eq!(code(&r), None);
    assert_eq!(ops(&r), ["Init(1)", "Borrow(1,2,mut)", "EndBorrow(2)", "Read(1)", "Return"]);
}
#[test]
fn projected_loan_is_not_exported_as_a_whole_owner_proof() {
    let r = main_record("pub fnc main(): int { let c = new C(); val x = borrow! c.x; return x; }");
    assert_eq!(code(&r), None);
    assert!(matches!(&r.mir, Err(why) if why.contains("field projection")));
}

// ---------------------------------------------------------------------------
// structured-cfg-v1: real CFG blocks for if / while / loop / for
// ---------------------------------------------------------------------------

/// Blocks as `id:[events]->[successors]` for compact golden assertions.
fn cfg(record: &Record) -> Vec<String> {
    let graph = record.mir.as_ref().unwrap_or_else(|why| panic!("mirUnavailable: {why}"));
    graph
        .blocks
        .iter()
        .enumerate()
        .map(|(id, b)| {
            let events: Vec<String> = b.events.iter().map(|(e, _)| e.short()).collect();
            let succ: Vec<String> = b.successors.iter().map(|s| s.to_string()).collect();
            format!("{id}:[{}]->[{}]", events.join(" "), succ.join(","))
        })
        .collect()
}

#[test]
fn if_else_exports_test_arm_false_and_join_blocks() {
    let r = main_record(
        "pub fnc main(): int { val int x = 1; val c = new C(); if x > 0 then val d = take! c; fi return 2; }",
    );
    assert_eq!(code(&r), None);
    assert_eq!(
        cfg(&r),
        [
            "0:[Init(1,copy) Init(2) Read(1)]->[1,2]",
            "1:[Read(2) Move(2->3)]->[3]",
            "2:[]->[3]",
            "3:[Return]->[]",
        ]
    );
}

#[test]
fn may_moved_owner_after_a_branch_is_rejected_by_both_sides() {
    let r = main_record(
        "pub fnc main(): int { val int x = 1; val c = new C(); if x > 0 then val d = take! c; fi return c.x; }",
    );
    assert_eq!(code(&r), Some("E_MOVED"));
    assert!(r.prefix_at_error);
    // The read after the join is the failing op, in the join block.
    assert_eq!(cfg(&r).last().unwrap(), "3:[Read(2) Return]->[]");
}

#[test]
fn arms_that_return_do_not_reach_the_join() {
    let r = main_record(
        "pub fnc main(): int { val int x = 1; if x > 0 then return 1; else return 2; fi }",
    );
    assert_eq!(code(&r), None);
    assert_eq!(
        cfg(&r),
        ["0:[Init(1,copy) Read(1)]->[1,2]", "1:[Return]->[]", "2:[Return]->[]"]
    );
}

#[test]
fn while_loop_has_header_body_back_edge_and_exit() {
    let r = main_record(
        "pub fnc main(): int { val int n = 3; val c = new C(); while n > 0 { val v = borrow! c; val y = v.x; } return c.x; }",
    );
    assert_eq!(code(&r), None);
    assert_eq!(
        cfg(&r),
        [
            "0:[Init(1,copy) Init(2)]->[1]",
            // header: condition; true -> body, false -> exit
            "1:[Read(1)]->[2,3]",
            // body: loan ends at its NLL last use, loop-local owner released, back-edge
            "2:[Borrow(2,3,shared) Read(2) Init(4,copy) EndBorrow(3) Drop(4)]->[1]",
            "3:[Read(2) Return]->[]",
        ]
    );
}

#[test]
fn break_and_continue_release_loop_locals_on_their_edges() {
    let r = main_record(
        "pub fnc main(): int { val int n = 3; loop { val int k = n; if k > 1 then break; fi continue; } return n; }",
    );
    assert_eq!(code(&r), None);
    let blocks = cfg(&r);
    // break edge releases `k` before leaving to the exit block.
    assert!(blocks.iter().any(|b| b.contains("Drop(2)]->[") && b.ends_with("->[]") == false), "{blocks:?}");
    assert!(blocks.last().unwrap().ends_with("[Read(1) Return]->[]"), "{blocks:?}");
}

#[test]
fn loop_without_break_has_no_exit_block() {
    let r = main_record("pub fnc main(): int { val int n = 3; loop { val int k = n; } }");
    let blocks = cfg(&r);
    assert!(blocks.iter().all(|b| !b.contains("Return")), "{blocks:?}");
}

#[test]
fn conditionally_moved_loop_local_owner_fails_closed() {
    let r = main_record(
        "pub fnc main(): int { val int n = 3; while n > 0 { val c = new C(); if n > 1 then val d = take! c; fi } return n; }",
    );
    assert!(matches!(&r.mir, Err(why) if why.contains("drop-if-owned")), "{:?}", r.mir);
}

#[test]
fn call_argument_temporaries_are_released_inside_loops() {
    let r = main_record(
        "fnc f(C p): int { return p.x; } pub fnc main(): int { val int n = 2; while n > 0 { val c = new C(); val a = f(c); } return n; }",
    );
    assert_eq!(code(&r), None);
    let body = cfg(&r)[2].clone();
    assert!(body.contains("Move(2->1000001) Drop(1000001)"), "{body}");
    // `c` is consumed on every path, so only `a` needs a per-iteration release.
    assert!(body.ends_with("Init(3,copy) Drop(3)]->[1]"), "{body}");
}

#[test]
fn rejection_in_a_loop_header_exports_the_repetition() {
    // The compiler rejects moving a pre-existing owner in a repeated loop
    // condition. The prefix keeps the repetition: the header re-runs or
    // exits, so canonical sees the second-iteration use after move.
    let r = main_record(
        "pub fnc main(): int { val amount = new C(); let count = 0; while take! amount > 0 { count = count + 1; } return count; }",
    );
    assert_eq!(code(&r), Some("E_MOVED"));
    let blocks = cfg(&r);
    assert!(blocks.iter().any(|b| b.starts_with("1:") && b.ends_with("->[1,2]")), "{blocks:?}");
}

#[test]
fn rejection_deep_in_a_loop_body_fails_closed() {
    let r = main_record(
        "pub fnc main(): int { val c = new C(); val int n = 1; while n > 0 { val d = take! c; } return 1; }",
    );
    assert_eq!(code(&r), Some("E_MOVED"));
    assert!(matches!(&r.mir, Err(why) if why.contains("loop body")), "{:?}", r.mir);
}
