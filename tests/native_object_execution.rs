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
fn native_symbol_visibility_matches_pub_and_private_scalar_contract() {
    let source = include_str!("fixtures/private-linkage.atto");
    let build = TemporaryBuild::new();
    let object = build.0.join("private-linkage.o");
    let bytes = compile(source).expect("compile canonical AttoLang guest fixture");
    assert!(bytes.starts_with(b"\x7fELF"), "native ELF object required");
    fs::write(&object, bytes).expect("write native guest object");

    // Check the object's actual external symbol table, not parser/MIR flags.
    let symbols = Command::new("nm")
        .args(["--extern-only", "--defined-only"])
        .arg(&object)
        .output()
        .expect("Linux native linker tests require nm");
    assert!(
        symbols.status.success(),
        "nm failed: {}",
        String::from_utf8_lossy(&symbols.stderr)
    );
    let symbol_text = String::from_utf8_lossy(&symbols.stdout);
    let global_names: Vec<&str> = symbol_text
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .collect();
    assert!(global_names.contains(&"answer"), "{symbol_text}");
    assert!(
        !global_names.contains(&"bump"),
        "private helper leaked into the native external ABI: {symbol_text}"
    );

    // A C translation unit can call the public function; its private callee
    // must still resolve internally, yielding the actual guest result 42.
    let public_driver = build.0.join("public.c");
    let public_executable = build.0.join("public");
    fs::write(
        &public_driver,
        "extern long answer(long); int main(void) { return (int)answer(41); }\n",
    )
    .expect("write public C ABI probe");
    let public_link = Command::new("cc")
        .arg(&public_driver)
        .arg(&object)
        .arg("-o")
        .arg(&public_executable)
        .output()
        .expect("link public ABI probe");
    assert!(
        public_link.status.success(),
        "public function did not link: {}",
        String::from_utf8_lossy(&public_link.stderr)
    );
    let guest = Command::new(&public_executable)
        .output()
        .expect("execute linked public/native guest");
    assert_eq!(guest.status.code(), Some(42), "{guest:?}");

    // A foreign C translation unit cannot name the private helper.
    let private_driver = build.0.join("private.c");
    let private_executable = build.0.join("private");
    fs::write(
        &private_driver,
        "extern long bump(long); int main(void) { return (int)bump(41); }\n",
    )
    .expect("write private ABI probe");
    let private_link = Command::new("cc")
        .arg(&private_driver)
        .arg(&object)
        .arg("-o")
        .arg(&private_executable)
        .output()
        .expect("link negative private ABI probe");
    assert!(
        !private_link.status.success(),
        "private helper was externally linkable: {}",
        String::from_utf8_lossy(&private_link.stderr)
    );
    assert!(!private_executable.exists());
}

#[test]
fn implicit_non_pub_main_remains_a_native_entry_point() {
    // The release profile keeps "main" as an executable root even without
    // pub. Preserve that existing entry point ABI while hiding other helpers.
    run_guest(
        "fnc main(): int { return helper(); } fnc helper(): int { return 42; }",
        42,
    );
}

#[test]
fn native_guest_executes_atto_immutable_default_binding_forms() {
    for body in [
        "int x = 7; int y = x * 6; return y;",
        "const x = 7; const y = x * 6; return y;",
        "const int x = 7; return x * 6;",
        "const x: int = 7; return x * 6;",
        "val x = 7; val y = x * 6; return y;",
        "int x = 7; const y = x * 6; return y;",
        "const x = add(20, 22); return x;",
    ] {
        let source = format!(
            "pub fnc main(): int {{ {body} }} fnc add(int a,int b): int {{ return a+b; }}"
        );
        run_guest(&source, 42);
    }
}

#[test]
fn native_guest_executes_byte_identical_atto_shared_scalar_fixture() {
    run_guest(include_str!("../contracts/atto-parity-v1/add.atto"), 42);
}

#[test]
fn native_guest_executes_canonical_define_fnc_entry() {
    for source in [
        "define pub fnc main(): int { return 42; }",
        "pub define fnc main(): int { return 42; }",
    ] {
        run_guest(source, 42);
    }
}

#[test]
fn native_guest_executes_explicit_mutable_scalar_rebindings() {
    for (body, expected) in [
        ("let int x = 7; x = x * 6; return x;", 42),
        ("mut int x = 7; x += 35; return x;", 42),
        ("let x: int = 43; x -= 1; return x;", 42),
        ("let x = 2; x = x * 2; x = x * 10; x += 2; return x;", 42),
        ("let x = 7; let y = x; x = 35; return x + y;", 42),
        ("let x = 7; const y = x; x += 35; return y + x - 7;", 42),
        ("let x = 1; const __atto_scalar_rebind_0 = 3; x = 39; return x + __atto_scalar_rebind_0;", 42),
        ("mut x = 45; x -= 3; return x;", 42),
    ] {
        let source = format!("pub fnc main(): int {{ {body} }}");
        run_guest(&source, expected);
    }
}

fn run_full_width_scalar_guest(source: &str, expected: i64) {
    let build = TemporaryBuild::new();
    let object = build.0.join("scalar.o");
    let driver = build.0.join("probe.c");
    let executable = build.0.join("probe");
    fs::write(&object, compile(source).expect("real Cranelift native scalar object"))
        .expect("persist native object");
    // Host-side C driver observes the full return value, unlike the
    // lossy 8-bit POSIX process exit status in historical scalar tests.
    let c_driver = r#"#include <stdio.h>
#include <stdint.h>
#include <inttypes.h>
extern int64_t scalar_value(void);
int main(void) {
    printf("%" PRId64 "\n", scalar_value());
    return 0;
}
"#;
    fs::write(&driver, c_driver).expect("write full-width native ABI probe");
    let link = Command::new("cc")
        .args([object.as_os_str(), driver.as_os_str()])
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("native linker available");
    assert!(link.status.success(), "native link failed: {}",
        String::from_utf8_lossy(&link.stderr));
    let guest = Command::new(&executable).output().expect("run native probe");
    assert!(guest.status.success(), "guest invocation failed: {}",
        String::from_utf8_lossy(&guest.stderr));
    let observed = String::from_utf8(guest.stdout).expect("UTF-8 decimal result");
    assert_eq!(observed.trim(), expected.to_string(),
        "full-width signed i64 result drifted");
}

#[test]
fn native_abi_preserves_full_signed_i64_after_mutable_rebind() {
    for (source, expected) in [
        ("pub fnc scalar_value(): int { let x = 9007199254740992; x += 1; return x; }",
         9_007_199_254_740_993_i64),
        ("pub fnc scalar_value(): int { mut x = -9223372036854775808; x += 1; return x; }",
         i64::MIN + 1),
        ("pub fnc scalar_value(): int { let x = 9007199254740993; let old = x; x -= 1; return old - x; }",
         1),
        ("pub fnc scalar_value(): int { mut x = -42; x = -x; return x; }", 42),
    ] {
        run_full_width_scalar_guest(source, expected);
    }
}
