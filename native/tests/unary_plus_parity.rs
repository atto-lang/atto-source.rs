//! Signed unary-plus v1: exact i64 identity, no implicit cast, evaluation once.
use oreslang_source_rs::{compile, compile_checked_i64, frontend, mir};

#[test]
fn unary_plus_preserves_integer_literals_and_nested_precedence() {
    let ast = frontend::parse("pub fnc main(): int { return +(+40) + +2; }")
        .expect("unary plus is valid on scalar expressions");
    assert!(matches!(ast.functions[0].expression, frontend::Expr::Add(..)));
    let literal = frontend::parse("pub fnc main(): int { return +42; }")
        .expect("positive signed literal");
    assert_eq!(literal.functions[0].expression, frontend::Expr::Integer(42));
    assert!(mir::lower(ast).is_ok(), "unary plus must preserve typed MIR");
}

#[test]
fn unary_plus_handles_calls_and_checked_codegen() {
    let source = "pub fnc main(): int { return +add(+19, +23); }\nfnc add(int left, int right): int { return left + right; }";
    assert!(!compile(source).expect("compile native object").is_empty());
    assert!(
        !compile_checked_i64(source)
            .expect("compile checked signed native object")
            .is_empty()
    );
}

#[test]
fn unary_plus_never_bypasses_i64_bounds_or_name_checking() {
    for source in [
        "pub fnc main(): int { return +9223372036854775808; }",
        "pub fnc main(): int { return ++9223372036854775808; }",
    ] {
        let err = compile(source).expect_err("out-of-range signed literal admitted");
        assert!(err.contains("integer_overflow"), "{source}: {err}");
    }
    let err = compile("pub fnc main(): int { return +not_declared; }")
        .expect_err("unbound signed operand admitted");
    assert!(err.contains("unknown_name"), "{err}");
}

#[cfg(target_os = "linux")]
#[test]
fn unary_plus_executes_full_i64_minimum_in_native_guest() {
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after UNIX epoch")
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "ores-rs-unary-plus-{}-{nonce}",
        std::process::id()
    )));
    fs::create_dir(&scratch.0).expect("create test directory");

    let source = "pub fnc scalar_value(): int { return +(-9223372036854775808); }";
    let object = compile_checked_i64(source).expect("checked codegen");
    assert!(object.starts_with(b"\x7fELF"), "expected native ELF object");
    let object_path = scratch.0.join("scalar.o");
    let driver_path = scratch.0.join("driver.c");
    let executable_path = scratch.0.join("guest");
    fs::write(&object_path, object).expect("write object");
    fs::write(
        &driver_path,
        "#include <stdint.h>\nextern int64_t scalar_value(void);\nint main(void) { return scalar_value() == INT64_MIN ? 0 : 91; }\n",
    )
    .expect("write i64 oracle");
    let linked = Command::new("cc")
        .arg(&object_path)
        .arg(&driver_path)
        .arg("-o")
        .arg(&executable_path)
        .output()
        .expect("native C compiler required");
    assert!(
        linked.status.success(),
        "native link failed: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let run = Command::new(&executable_path)
        .output()
        .expect("execute compiled guest");
    assert_eq!(
        run.status.code(),
        Some(0),
        "native guest result mismatch: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}
