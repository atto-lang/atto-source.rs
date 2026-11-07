//! End-to-end tests of actual Oreslang -> Cranelift -> linked native code.
//! Linux CI runs the object with cc; these are not merely source checks.
//! In particular, high i64 precision tests would catch accidental f64 lowering.
#![cfg(target_os = "linux")]

use oreslang_source_rs::compile;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TemporaryBuild(PathBuf);

impl TemporaryBuild {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oresc-compiler-e2e-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("cannot create isolated compiler test dir");
        Self(path)
    }
}

impl Drop for TemporaryBuild {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_guest(source: &str, expected_exit: i32) {
    let build = TemporaryBuild::new();
    let object = build.0.join("guest.o");
    let executable = build.0.join("guest");
    let bytes = compile(source)
        .unwrap_or_else(|error| panic!("native object compilation failed: {error}"));
    assert!(bytes.starts_with(b"\x7fELF"), "expected ELF machine object");
    fs::write(&object, bytes).expect("cannot persist compiled object");
    let link = Command::new("cc")
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("C compiler/linker is required for Linux native compiler tests");
    assert!(
        link.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    let guest = Command::new(&executable)
        .output()
        .expect("native Oreslang guest could not run");
    assert!(
        guest.status.success() || guest.status.code().is_some(),
        "guest terminated by signal: {:?}",
        guest.status
    );
    assert_eq!(
        guest.status.code(),
        Some(expected_exit),
        "unexpected result from linked Oreslang guest: {source:?}; stderr={}",
        String::from_utf8_lossy(&guest.stderr)
    );
}

#[test]
fn true_native_guest_preserves_i64_above_binary64_mantissa() {
    run_guest(
        "pub fnc main(): int { return 9007199254740993 - 9007199254740992; }",
        1,
    );
}

#[test]
fn true_native_guest_preserves_signed_min_boundary() {
    run_guest(
        "pub fnc main(): int { return -9223372036854775808 + 9223372036854775807 + 43; }",
        42,
    );
}

#[test]
fn true_native_guest_preserves_signed_max_boundary() {
    run_guest(
        "pub fnc main(): int { return 9223372036854775807 - 9223372036854775765; }",
        42,
    );
}

#[test]
fn true_native_guest_preserves_negative_multiply_and_negate() {
    run_guest("pub fnc main(): int { return -((-7)*6); }", 42);
}

#[test]
fn true_native_guest_preserves_parenthesized_precedence() {
    run_guest("pub fnc main(): int { return (3+4)*6; }", 42);
}

#[test]
fn true_native_guest_preserves_left_associative_subtraction() {
    run_guest("pub fnc main(): int { return 20-5-3; }", 12);
}

#[test]
fn true_native_guest_preserves_forward_calls() {
    run_guest(
        "pub fnc main(): int { return answer(20,22); } fnc answer(int a,int b): int { return a+b; }",
        42,
    );
}

#[test]
fn true_native_guest_preserves_local_initializers() {
    run_guest(
        "pub fnc main(): int { const factor: int = 7; const six: int = 6; return factor*six; }",
        42,
    );
}

#[test]
fn native_guest_nested_forward_calls_preserve_arguments() {
    run_guest(
        "pub fnc main(): int { return twice(plus_one(20)); } fnc twice(int x): int { return x*2; } fnc plus_one(int x): int { return x+1; }",
        42,
    );
}

#[test]
fn native_guest_call_from_const_initializer() {
    run_guest(
        "pub fnc main(): int { const x: int = add(20,22); return x; } fnc add(int a,int b): int { return a+b; }",
        42,
    );
}

#[test]
fn native_guest_lexical_comments_do_not_change_return_value() {
    run_guest(
        "/* begin /* nested */ ignored */ pub fnc main(): int { // inline\n return 6 * 7; }",
        42,
    );
}

#[test]
fn native_guest_exact_minimum_boundary_delta() {
    run_guest(
        "pub fnc main(): int { return (-9223372036854775808 + 9223372036854775807) + 43; }",
        42,
    );
}

#[test]
fn native_guest_exact_high_precision_subtraction_with_underscores() {
    run_guest(
        "pub fnc main(): int { return 9_007_199_254_741_034 - 9_007_199_254_740_992; }",
        42,
    );
}

#[test]
fn native_guest_nested_negative_signs_preserve_meaning() {
    run_guest(
        "pub fnc main(): int { return -(-((6*7))); }",
        42,
    );
}

#[test]
fn native_guest_zero_exit_is_not_silently_changed() {
    run_guest("pub fnc main(): int { return 42-42; }", 0);
}

#[test]
fn native_guest_parameter_negation_compiles_to_signed_result() {
    run_guest(
        "pub fnc main(): int { return flip(-42); } fnc flip(int x): int { return -x; }",
        42,
    );
}

#[test]
fn shared_llvm_oresllm_scalar_corpus_executes_as_rust_native_guests() {
    // Every guest source is byte-identical to the shared LLVM v1 .ores file;
    // the .atto extension is canonical for new AttoLang guest tests.
    for (name, source) in [
        ("oresllm-precedence", include_str!("fixtures/oresllm_scalar_v1/oresllm-precedence.atto")),
        ("oresllm-scalar-gradient", include_str!("fixtures/oresllm_scalar_v1/oresllm-scalar-gradient.atto")),
        ("oresllm-signed-product", include_str!("fixtures/oresllm_scalar_v1/oresllm-signed-product.atto")),
        ("oresllm-grouped-negative", include_str!("fixtures/oresllm_scalar_v1/oresllm-grouped-negative.atto")),
        ("oresllm-nested-precedence", include_str!("fixtures/oresllm_scalar_v1/oresllm-nested-precedence.atto")),
        ("oresllm-max-minus-one", include_str!("fixtures/oresllm_scalar_v1/oresllm-max-minus-one.atto")),
        ("oresllm-zero-product", include_str!("fixtures/oresllm_scalar_v1/oresllm-zero-product.atto")),
        ("oresllm-nested-arithmetic", include_str!("fixtures/oresllm_scalar_v1/oresllm-nested-arithmetic.atto")),
        ("oresllm-subtract-self", include_str!("fixtures/oresllm_scalar_v1/oresllm-subtract-self.atto")),
        ("oresllm-zero-add", include_str!("fixtures/oresllm_scalar_v1/oresllm-zero-add.atto")),
        ("oresllm-signed-subtract", include_str!("fixtures/oresllm_scalar_v1/oresllm-signed-subtract.atto")),
        ("oresllm-left-associative", include_str!("fixtures/oresllm_scalar_v1/oresllm-left-associative.atto")),
        ("oresllm-zero-times-max", include_str!("fixtures/oresllm_scalar_v1/oresllm-zero-times-max.atto")),
        ("oresllm-over-2pow53", include_str!("fixtures/oresllm_scalar_v1/oresllm-over-2pow53.atto")),
        ("oresllm-min-max-cancel", include_str!("fixtures/oresllm_scalar_v1/oresllm-min-max-cancel.atto")),
        ("oresllm-near-square", include_str!("fixtures/oresllm_scalar_v1/oresllm-near-square.atto")),
        ("oresllm-over53-cancel", include_str!("fixtures/oresllm_scalar_v1/oresllm-over53-cancel.atto")),
    ] {
        // Parse -> MIR -> Cranelift -> cc link -> native guest exit 42.
        assert!(source.starts_with("pub fnc main"), "{name}");
        run_guest(source, 42);
    }
}
