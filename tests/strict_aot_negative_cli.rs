//! Real compiler-backed strict-AOT rejection on the Rust/Cranelift scalar subset.
//! This does not certify actors, GC, Proxy or five-backend parity.
#![forbid(unsafe_code)]
use oreslang_source_rs::{frontend, mir};
use std::{collections::BTreeSet, ffi::OsStr, fs, path::{Path, PathBuf},
    process::{Command, Output, Stdio}, sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage { Parse, Mir }
struct Case { name: &'static str, source: &'static str, stage: Stage, class: &'static str }
const CASES: &[Case] = &[
    Case { name: "unknown-value.atto", source: include_str!("fixtures/strict_aot_negative/unknown-value.atto"), stage: Stage::Mir, class: "unknown_name" },
    Case { name: "unknown-callee.atto", source: include_str!("fixtures/strict_aot_negative/unknown-callee.atto"), stage: Stage::Mir, class: "unknown_name" },
    Case { name: "call-arity.atto", source: include_str!("fixtures/strict_aot_negative/call-arity.atto"), stage: Stage::Mir, class: "call_arity" },
    Case { name: "duplicate-function.atto", source: include_str!("fixtures/strict_aot_negative/duplicate-function.atto"), stage: Stage::Mir, class: "duplicate_declaration" },
    Case { name: "duplicate-parameter.atto", source: include_str!("fixtures/strict_aot_negative/duplicate-parameter.atto"), stage: Stage::Mir, class: "duplicate_declaration" },
    Case { name: "duplicate-local.atto", source: include_str!("fixtures/strict_aot_negative/duplicate-local.atto"), stage: Stage::Mir, class: "duplicate_declaration" },
    Case { name: "forward-local.atto", source: include_str!("fixtures/strict_aot_negative/forward-local.atto"), stage: Stage::Mir, class: "unknown_name" },
    Case { name: "dead-unknown-callee.atto", source: include_str!("fixtures/strict_aot_negative/dead-unknown-callee.atto"), stage: Stage::Mir, class: "unknown_name" },
    Case { name: "overflow-positive.atto", source: include_str!("fixtures/strict_aot_negative/overflow-positive.atto"), stage: Stage::Parse, class: "integer_overflow" },
    Case { name: "overflow-negative.atto", source: include_str!("fixtures/strict_aot_negative/overflow-negative.atto"), stage: Stage::Parse, class: "integer_overflow" },
    Case { name: "nested-comment-unclosed.atto", source: include_str!("fixtures/strict_aot_negative/nested-comment-unclosed.atto"), stage: Stage::Parse, class: "unterminated_comment" },
    Case { name: "invalid-separator.atto", source: include_str!("fixtures/strict_aot_negative/invalid-separator.atto"), stage: Stage::Parse, class: "syntax_error" },
    Case { name: "percent-operator.atto", source: include_str!("fixtures/strict_aot_negative/percent-operator.atto"), stage: Stage::Parse, class: "syntax_error" },
    Case { name: "caret-operator.atto", source: include_str!("fixtures/strict_aot_negative/caret-operator.atto"), stage: Stage::Parse, class: "syntax_error" },
    Case { name: "double-caret-operator.atto", source: include_str!("fixtures/strict_aot_negative/double-caret-operator.atto"), stage: Stage::Parse, class: "syntax_error" },
    Case { name: "eval-absent.atto", source: include_str!("fixtures/strict_aot_negative/eval-absent.atto"), stage: Stage::Mir, class: "unknown_name" },
    Case { name: "cpu-block-unlowered.atto", source: include_str!("fixtures/strict_aot_negative/cpu-block-unlowered.atto"), stage: Stage::Parse, class: "execution_domain_unsupported" },
    Case { name: "gpu-block-unlowered.atto", source: include_str!("fixtures/strict_aot_negative/gpu-block-unlowered.atto"), stage: Stage::Parse, class: "execution_domain_unsupported" },
];
const POSITIVE: &str = include_str!("fixtures/strict_aot_negative/positive-scalar.atto");
static COUNTER: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let path = std::env::temp_dir().join(format!(
            "atto-strict-cli-{tag}-{}-{id}-{now}", std::process::id()));
        fs::create_dir(&path).expect("create isolated scratch");
        Self(path)
    }
    fn file(&self, name: &str) -> PathBuf { self.0.join(name) }
}
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
fn repo_root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf() }
fn fixture_path(name: &str, expected: &str) -> PathBuf {
    assert!(name.ends_with(".atto") && !name.contains('/') && !name.contains('\\'));
    let dir = repo_root().join("tests/fixtures/strict_aot_negative");
    let dir_meta = fs::symlink_metadata(&dir).expect("fixture directory missing");
    assert!(dir_meta.is_dir() && !dir_meta.file_type().is_symlink());
    let path = dir.join(name);
    let meta = fs::symlink_metadata(&path).expect("fixture missing");
    assert!(meta.is_file() && !meta.file_type().is_symlink());
    assert!(meta.len() > 0 && meta.len() <= 16_384);
    assert_eq!(fs::read(&path).expect("read fixture"), expected.as_bytes(),
        "the compiled-in canonical fixture differs from disk");
    path
}
fn before_native_ir(case: &Case) -> String {
    match frontend::parse(case.source) {
        Err(err) => {
            assert_eq!(case.stage, Stage::Parse, "wrong rejection stage: {}", case.name);
            assert_eq!(err.class, case.class, "wrong diagnostic class: {}", case.name);
            err.to_string()
        }
        Ok(program) => {
            assert_eq!(case.stage, Stage::Mir, "wrong rejection stage: {}", case.name);
            let err = mir::lower(program).expect_err("invalid guest lowered into MIR");
            assert_eq!(err.class, case.class, "wrong diagnostic class: {}", case.name);
            err.to_string()
        }
    }
}
fn run(args: &[&OsStr]) -> Output {
    let result = Command::new(env!("CARGO_BIN_EXE_oresc"))
        .args(args).current_dir(repo_root()).stdin(Stdio::null())
        .output().expect("real oresc compiler must launch");
    assert!(result.stderr.len() <= 16_384 && result.stdout.len() <= 16_384,
        "unbounded or unexpected compiler output");
    result
}
fn rejected(result: Output, diagnostic: &str, name: &str, mode: &str) {
    assert_eq!(result.status.code(), Some(1),
        "wrong exit status on {name} / {mode}");
    assert!(result.stdout.is_empty(), "rejected {name} published stdout/IR in {mode}");
    let stderr = std::str::from_utf8(&result.stderr).expect("UTF-8 diagnostic");
    assert_eq!(stderr.trim_end_matches('\n'), format!("oresc: {diagnostic}"),
        "wrong rejection reason on {name} / {mode}");
}
fn verify(case: &Case) {
    let input = fixture_path(case.name, case.source);
    let diagnostic = before_native_ir(case);
    let tmp = Scratch::new(case.name.trim_end_matches(".atto"));
    let object = tmp.file("invalid.o");
    let effects = tmp.file("invalid.effects.json");
    let old_object = tmp.file("unchanged.o");
    let old_effects = tmp.file("unchanged.effects.json");
    fs::write(&old_object, b"UNCHANGED_OBJECT").unwrap();
    fs::write(&old_effects, b"UNCHANGED_EFFECTS").unwrap();
    rejected(run(&[OsStr::new("--strict-aot"), OsStr::new("--check"), input.as_os_str()]),
        &diagnostic, case.name, "check");
    rejected(run(&[OsStr::new("--strict-aot"), input.as_os_str(), object.as_os_str()]),
        &diagnostic, case.name, "object");
    assert!(!object.exists(), "native output created for rejected guest");
    rejected(run(&[OsStr::new("--strict-aot"), OsStr::new("--compile-with-effects"),
        input.as_os_str(), object.as_os_str(), effects.as_os_str()]),
        &diagnostic, case.name, "paired");
    assert!(!object.exists() && !effects.exists(), "paired artifacts created for rejected guest");
    rejected(run(&[OsStr::new("--strict-aot"), input.as_os_str(), old_object.as_os_str()]),
        &diagnostic, case.name, "preexisting-object");
    rejected(run(&[OsStr::new("--strict-aot"), OsStr::new("--compile-with-effects"),
        input.as_os_str(), old_object.as_os_str(), old_effects.as_os_str()]),
        &diagnostic, case.name, "preexisting-paired");
    assert_eq!(fs::read(&old_object).unwrap(), b"UNCHANGED_OBJECT");
    assert_eq!(fs::read(&old_effects).unwrap(), b"UNCHANGED_EFFECTS");
    for mode in ["--effects", "--static-facts", "--cfg-ir", "--safety-ir"] {
        rejected(run(&[OsStr::new("--strict-aot"), OsStr::new(mode), input.as_os_str()]),
            &diagnostic, case.name, mode);
    }
}
macro_rules! case_test {
    ($test:ident, $index:expr) => { #[test] fn $test() { verify(&CASES[$index]); } };
}
case_test!(reject_unknown_value, 0);
case_test!(reject_unknown_callee, 1);
case_test!(reject_call_arity, 2);
case_test!(reject_duplicate_function, 3);
case_test!(reject_duplicate_parameter, 4);
case_test!(reject_duplicate_local, 5);
case_test!(reject_forward_local, 6);
case_test!(reject_dead_unknown_callee, 7);
case_test!(reject_overflow_positive, 8);
case_test!(reject_overflow_negative, 9);
case_test!(reject_nested_comment_unclosed, 10);
case_test!(reject_invalid_separator, 11);
case_test!(reject_percent_operator, 12);
case_test!(reject_caret_operator, 13);
case_test!(reject_double_caret_operator, 14);
case_test!(reject_eval_absent, 15);
case_test!(reject_cpu_block_unlowered, 16);
case_test!(reject_gpu_block_unlowered, 17);

#[test]
fn inventory_is_unique_and_bounded_with_both_rejection_stages() {
    assert_eq!(CASES.len(), 18);
    let mut names = BTreeSet::new();
    let mut contents = BTreeSet::new();
    let mut classes = BTreeSet::new();
    for case in CASES {
        assert!(names.insert(case.name), "duplicate source path");
        assert!(contents.insert(case.source), "duplicate source body");
        classes.insert(case.class);
        fixture_path(case.name, case.source);
    }
    assert!(CASES.iter().any(|x| x.stage == Stage::Parse));
    assert!(CASES.iter().any(|x| x.stage == Stage::Mir));
    for class in ["unknown_name", "call_arity", "duplicate_declaration",
                  "integer_overflow", "unterminated_comment", "execution_domain_unsupported"] {
        assert!(classes.contains(class), "missing diagnostic class {class}");
    }
}
#[test]
fn positive_canonical_atto_emits_real_native_object_and_effects() {
    let input = fixture_path("positive-scalar.atto", POSITIVE);
    let tmp = Scratch::new("positive");
    let object = tmp.file("positive.o");
    let effects = tmp.file("positive.effects.json");
    let check = run(&[OsStr::new("--strict-aot"), OsStr::new("--check"), input.as_os_str()]);
    assert!(check.status.success(), "valid source rejected: {}", String::from_utf8_lossy(&check.stderr));
    assert_eq!(check.stdout, b"OK: 2 function(s)\n");
    let out = run(&[OsStr::new("--strict-aot"), OsStr::new("--compile-with-effects"),
        input.as_os_str(), object.as_os_str(), effects.as_os_str()]);
    assert!(out.status.success(), "native compilation failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(fs::metadata(&object).expect("object not produced").len() > 16);
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&effects).unwrap())
        .expect("compiler effects must be valid JSON");
    assert!(json.get("functions").and_then(serde_json::Value::as_array).is_some());
    // This test does NOT link or execute the native guest.
}
#[test]
fn incorrect_cli_invocation_never_counts_as_expected_semantic_rejection() {
    let bad = run(&[OsStr::new("--strict-aot")]);
    assert!(!bad.status.success());
    assert!(!std::str::from_utf8(&bad.stderr).unwrap().contains("unknown_name"));
    let input = fixture_path("positive-scalar.atto", POSITIVE);
    let good = run(&[OsStr::new("--strict-aot"), OsStr::new("--check"), input.as_os_str()]);
    assert!(good.status.success(), "positive control proves real driver is invoked");
}
