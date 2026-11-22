//! Honest std/time admission gate for the independent browser Rust compiler.
//! Source-file presence is not proof of executable guest class/method support.
use atto_compiler::compile;
use std::path::Path;

const SOURCE: &str = include_str!("../stdlib/std/time.atto");

#[test]
fn full_std_time_cannot_silently_generate_a_guest_artifact() {
    assert!(SOURCE.contains("define module time as"));
    assert!(SOURCE.contains("define pub class Duration as"));
    let diagnostic = compile(SOURCE).expect_err(
        "class-based std/time must remain fail-closed until guest class ABI exists");
    assert!(!diagnostic.code.is_empty());
}

#[test]
fn importing_std_time_must_not_fallback_to_untrusted_project_source() {
    let guest = r#"import module time from "std/time";
pub fnc main(): int { return time.seconds(1); }"#;
    let diagnostic = compile(guest).expect_err(
        "stdlib import must be admitted and linked before producing Wasm bytes");
    assert!(!diagnostic.code.is_empty());
}

#[test]
fn scalar_guest_compilation_remains_available_as_positive_control() {
    // This is a compiler positive control, NOT a time guest program.
    let wasm = atto_compiler::compile_with_path(
        "pub fnc main(): int { return 42; }",
        Path::new("time_positive_control.atto"))
        .expect("scalar compiler must still emit a real Wasm artifact");
    assert!(wasm.len() >= 8);
    assert_eq!(&wasm[..4], &[0x00, 0x61, 0x73, 0x6d]);
}
