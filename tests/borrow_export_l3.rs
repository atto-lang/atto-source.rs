//! L3 borrow-conformance exporter (`borrow_export`, profile straight-line-v1).
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
        ("pub fnc main(): int { val int x = 1; if x > 0 then return 1; fi return 2; }", "branching"),
        ("pub fnc main(): int { val c = new C(); return c.get(); }", "method call"),
        ("pub fnc main(): int { let int x = 1; x = 2; return x; }", "reassignment"),
        ("pub fnc main(): int { let int i = 0; while i < 3 { i = i + 1; } return i; }", "loops"),
    ] {
        let r = main_record(body);
        match &r.mir {
            Err(why) => assert!(why.contains(reason), "{body}: {why}"),
            Ok(_) => panic!("{body}: exported a graph outside straight-line-v1"),
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
    let r = main_record("pub fnc main(): int { val c = new C(); return missing; }");
    assert_eq!(code(&r), Some("E_UNBOUND"));
    assert!(matches!(&r.mir, Err(why) if why.contains("non-ownership")));
}

#[test]
fn admitted_shallow_copy_does_not_export_unproved_ownership_graph() {
    let r = main_record("pub fnc main(): int { val c = new C(); val d = copy! c; return d.x; }");
    assert_eq!(code(&r), None);
    assert!(!r.prefix_at_error);
    assert!(matches!(&r.mir, Err(why) if why.contains("shallow class copy witness")));
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
