//! Shared Java/LLVM typed ternary contract through the actual Rust compiler.
//! Source vectors are stored in canonical .atto guest fixtures.
use oreslang_source_rs::{
    cfg, compile, compile_checked_i64, effects, flow, frontend, mir, safety,
    static_facts,
};

fn checked(source: &str) -> Result<mir::Module, String> {
    let program = frontend::parse(source).map_err(|error| error.to_string())?;
    mir::lower(program).map_err(|error| error.to_string())
}

fn assert_reject(source: &str, class: &str) {
    let error = compile(source).expect_err("invalid guest admitted");
    assert!(
        error.starts_with(class),
        "expected {class}, got {error} for {source:?}"
    );
}

#[cfg(target_os = "linux")]
fn run_guest(source: &str, checked_arithmetic: bool) -> std::process::ExitStatus {
    use std::{
        fs,
        path::PathBuf,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_nanos();
    let dir = Temp(std::env::temp_dir().join(format!(
        "ores-ternary-{}-{nonce}", std::process::id()
    )));
    fs::create_dir(&dir.0).expect("create guest test dir");
    let object = dir.0.join("guest.o");
    let executable = dir.0.join("guest");
    let compiled = if checked_arithmetic {
        compile_checked_i64(source)
    } else {
        compile(source)
    }
    .unwrap_or_else(|error| panic!("native compile: {error} for {source:?}"));
    assert!(compiled.starts_with(b"\x7fELF"));
    fs::write(&object, compiled).expect("write native guest object");
    let linked = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("Linux C linker required");
    assert!(
        linked.status.success(),
        "native link error: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    Command::new(&executable)
        .current_dir(&dir.0)
        .status()
        .expect("execute native AttoLang guest")
}

#[test]
fn shared_contract_vectors_match_guest_fixtures_and_diagnostics() {
    let contract: serde_json::Value = serde_json::from_str(
        include_str!("../contracts/v1/conditional-expression-v1.json"),
    )
    .unwrap();
    assert_eq!(contract["schema"], "oreslang.conditional-expression.v1");
    let fixtures = [
        ("literal_true", include_str!("fixtures/conditional_v1/literal_true.atto")),
        ("literal_false", include_str!("fixtures/conditional_v1/literal_false.atto")),
        ("right_associative", include_str!("fixtures/conditional_v1/right_associative.atto")),
        (
            "arithmetic_condition_precedence",
            include_str!("fixtures/conditional_v1/arithmetic_condition_precedence.atto"),
        ),
        (
            "reject_integer_truthiness",
            include_str!("fixtures/conditional_v1/reject_integer_truthiness.atto"),
        ),
    ];
    let vectors = contract["portable_scalar_vectors"].as_array().unwrap();
    assert_eq!(fixtures.len(), vectors.len());
    for vector in vectors {
        let name = vector["id"].as_str().unwrap();
        let source = vector["source"].as_str().unwrap();
        let fixture = fixtures.iter().find(|(id, _)| *id == name).unwrap().1;
        assert_eq!(fixture.trim_end(), source, "drift for {name}");
        if let Some(class) = vector["reject"].as_str() {
            assert_reject(fixture, class);
        } else {
            checked(fixture).expect("canonical condition must type-check");
            #[cfg(target_os = "linux")]
            {
                let expected = vector["result"].as_i64().unwrap() as i32;
                let observed = run_guest(fixture, false);
                assert_eq!(observed.code(), Some(expected), "{name}: {observed:?}");
            }
        }
    }
}

#[test]
fn checks_both_branches_and_rejects_wrong_scalar_types() {
    for (fixture, class) in [
        (include_str!("fixtures/conditional_v1/unselected_missing_name.atto"), "unknown_name"),
        (include_str!("fixtures/conditional_v1/branch_type_mismatch.atto"), "branch_type_mismatch"),
        (
            include_str!("fixtures/conditional_v1/equality_type_mismatch.atto"),
            "operand_type_mismatch",
        ),
        (include_str!("fixtures/conditional_v1/arithmetic_on_bool.atto"), "operand_not_int"),
        (include_str!("fixtures/conditional_v1/bool_return.atto"), "return_type_mismatch"),
        (include_str!("fixtures/conditional_v1/bool_local.atto"), "local_type_mismatch"),
        (include_str!("fixtures/conditional_v1/bool_argument.atto"), "argument_not_int"),
    ] {
        assert_reject(fixture, class);
    }
}

#[test]
fn refuses_false_linear_effect_certification_for_typed_branches() {
    let src = include_str!("fixtures/conditional_v1/literal_true.atto");
    let m = checked(src).unwrap();
    assert!(mir::uses_typed_conditionals(&m));
    assert!(effects::export_effect_ir_checked(&m).is_err());
    assert_eq!(effects::export_effect_ir(&m)["effects_complete"], false);
    assert!(cfg::export_cfg_v2(&m, Some(32)).is_err());
    assert!(flow::export_scalar_flow(&m, 32).is_err());
    assert_eq!(safety::export_safety_ir(&m, Some(32))["effects_complete"], false);
    assert!(static_facts::export(&m, src).is_err());
}

#[test]
fn guards_nested_conditional_depth_before_unbounded_recursion() {
    let src = format!(
        "pub fnc main(): int {{ return {}42; }}",
        "true ? 1 : ".repeat(frontend::MAX_EXPRESSION_DEPTH + 1)
    );
    let error = frontend::parse(&src).unwrap_err();
    assert_eq!(error.class, "resource_limit");
}

#[test]
#[cfg(target_os = "linux")]
fn boolean_equality_and_nested_condition_execute_natively() {
    for src in [
        include_str!("fixtures/conditional_v1/bool_equality.atto"),
        include_str!("fixtures/conditional_v1/nested_bool_result.atto"),
    ] {
        assert_eq!(run_guest(src, false).code(), Some(42));
    }
}

#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn unselected_checked_overflow_must_not_execute() {
    use std::os::unix::process::ExitStatusExt;
    let skipped = include_str!("fixtures/conditional_v1/lazy_nonselected_overflow.atto");
    assert_eq!(run_guest(skipped, true).code(), Some(42));
    let selected = include_str!("fixtures/conditional_v1/selected_overflow.atto");
    assert_eq!(run_guest(selected, true).signal(), Some(4));
}

#[test]
#[cfg(target_os = "linux")]
fn unicode_line_comments_preserve_native_ternary_guest_compilation() {
    for separator in ["\u{0085}", "\u{2028}", "\u{2029}"] {
        let guest = format!(
            "// inactive ? branch{separator}/* false ? 7 : 1 */\n\
             pub fnc main(): int {{ return true ? 42 : 7; }}"
        );
        let status = run_guest(&guest, true);
        assert_eq!(status.code(), Some(42), "line separator {separator:?}");
    }
}
