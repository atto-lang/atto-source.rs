//! Scoped executable Rust/Cranelift checked-i64 call-boundary regression.
//! This proves only the opt-in checked-i64 *native trap* on Linux/x86-64;
//! not an actor error, guest Error object, default arithmetic policy,
//! optimized LLVM/Graal parity, or completed cross-backend conformance.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use oreslang_source_rs::compile_checked_i64;
use std::{
    fs,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, ExitStatus},
    time::{SystemTime, UNIX_EPOCH},
};

struct Work(PathBuf);
impl Work {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oresc-checked-call-boundary-{}-{nonce}", std::process::id()
        ));
        fs::create_dir_all(&path).expect("create private native build directory");
        Self(path)
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn actual_native_status(source: &str) -> ExitStatus {
    let work = Work::new();
    let obj = work.0.join("guest.o");
    let binary = work.0.join("guest");
    let bytes = compile_checked_i64(source)
        .unwrap_or_else(|e| panic!("checked compile failed: {e}; source: {source}"));
    assert!(bytes.starts_with(b"\x7fELF"), "required real ELF object");
    fs::write(&obj, bytes).expect("write native object");
    let linked = Command::new("cc")
        .arg(&obj)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("run system C linker");
    assert!(
        linked.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    Command::new(binary)
        .current_dir(&work.0)
        .status()
        .expect("execute linked native guest")
}

#[test]
fn dynamic_interprocedural_overflow_is_never_silently_wrapped_or_dce_removed() {
    const ADD: &str = "fnc add(int a,int b): int { return a+b; }";
    const MULTIPLY: &str = "fnc mul(int a,int b): int { return a*b; }";
    let cases = [
        // Actual signed overflow occurs inside a function with opaque params.
        format!("{ADD} pub fnc main(): int {{ return add(9223372036854775807,1); }}"),
        format!("{ADD} pub fnc main(): int {{ return add(9223372036854775807,1)*0; }}"),
        // Eager call arguments must execute despite caller ignoring the result.
        format!("{ADD} fnc first(int a,int b): int {{ return a; }} \
            pub fnc main(): int {{ return first(42,add(9223372036854775807,1)); }}"),
        // An unused eager local initializer must still retain its trapping edge.
        format!("{ADD} pub fnc main(): int {{ const unused: int = add(9223372036854775807,1); return 42; }}"),
        // The multiplication overflow must not be optimized into a zero result.
        format!("{MULTIPLY} pub fnc main(): int {{ return mul(3037000500,3037000500)*0; }}"),
        // Helper -> helper -> main passes an overflow through two call boundaries.
        format!("{ADD} fnc wrapper(int x): int {{ return add(x,1); }} \
            pub fnc main(): int {{ return wrapper(9223372036854775807); }}"),
        // Signed minimum overflow through a function parameter.
        "fnc neg(int x): int { return -x; } pub fnc main(): int { return neg(-9223372036854775808); }".to_string(),
    ];
    for (index, source) in cases.iter().enumerate() {
        let result = actual_native_status(source);
        assert_eq!(
            result.signal(),
            Some(4),
            "case {index}: expected SIGILL native signed overflow trap, got {result:?}"
        );
    }
}

#[test]
fn full_width_safe_parameters_and_matched_boundaries_still_execute() {
    let cases = [
        ("fnc add(int a,int b): int { return a+b; } \
          pub fnc main(): int { return add(40,2); }", 42),
        ("fnc add(int a,int b): int { return a+b; } \
          pub fnc main(): int { return add(9223372036854775807,0)-9223372036854775807+42; }", 42),
        ("fnc sub(int a,int b): int { return a-b; } \
          pub fnc main(): int { return sub(-9223372036854775808,0)-(-9223372036854775808)+42; }", 42),
        ("fnc mul(int a,int b): int { return a*b; } \
          pub fnc main(): int { return mul(3037000499,3037000499)-9223372030926249001+42; }", 42),
    ];
    for (source, expected) in cases {
        let result = actual_native_status(source);
        assert_eq!(result.code(), Some(expected), "{source}: {result:?}");
    }
}
