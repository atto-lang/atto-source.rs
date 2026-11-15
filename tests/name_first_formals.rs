//! Canonical name-first scalar formals preserve the existing direct native i64 ABI.
//! This does not admit inferred, generic, function-valued, or actor parameters.
use oreslang_source_rs::{compile, compile_with_effect_ir, frontend, mir};

const MIXED: &str = "pub fnc native_value(): int { return add(19, 23); }\nfnc add(left: int, int right): int { return left + right; }";

#[test]
fn mixed_formals_preserve_parameter_order_and_mir_names() {
    let ast = frontend::parse(MIXED).expect("name-first and type-first must parse");
    assert_eq!(ast.functions[1].params, ["left", "right"]);
    let lowered = mir::lower(ast).expect("typed ordered parameter names are valid MIR");
    assert_eq!(lowered.functions[1].params, ["left", "right"]);

    let (object, facts) = compile_with_effect_ir(MIXED).expect("emit native object and effects");
    assert!(!object.is_empty());
    assert_eq!(facts["functions"][0]["operations"][2]["target"], "add");
    assert!(!compile(MIXED).expect("plain native object").is_empty());
}

#[test]
fn name_first_forward_calls_and_multiple_parameters_are_admitted() {
    let source = "pub fnc main(): int { return f(20, 22); }\nfnc f(a: int, b: int): int { return a + b; }";
    assert!(compile(source).is_ok(), "forward calls must preserve the same ABI");
}

#[test]
fn types_must_remain_explicit_and_duplicate_names_are_rejected() {
    for source in [
        "fnc f(value): int { return value; }",
        "fnc f(value:): int { return value; }",
        "fnc f(value: value): int { return value; }",
        "fnc f(int): int { return 42; }",
        "fnc f(value: int, other): int { return value; }",
    ] {
        let err = frontend::parse(source).expect_err("untyped parameter admitted");
        assert_eq!(err.class, "syntax_error", "{source}: {err}");
    }

    for source in [
        "fnc f(value: int, value: int): int { return value; }",
        "fnc f(int value, value: int): int { return value; }",
    ] {
        let err = compile(source).expect_err("duplicate parameter admitted");
        assert!(err.contains("duplicate_declaration"), "{source}: {err}");
    }
}

#[test]
fn call_arity_is_identical_for_both_parameter_spellings() {
    let source = "pub fnc main(): int { return f(42); }\nfnc f(a: int, int b): int { return a+b; }";
    let err = compile(source).expect_err("missing argument admitted");
    assert!(err.contains("call_arity"), "{err}");
}

#[cfg(target_os = "linux")]
#[test]
fn mixed_formals_link_and_execute_real_native_object() {
    use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after UNIX epoch")
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "ores-rs-name-first-{}-{nonce}",
        std::process::id()
    )));
    fs::create_dir(&scratch.0).expect("create isolated native test directory");
    let object = compile(MIXED).expect("compile scalar source");
    assert!(object.starts_with(b"\x7fELF"), "expected native ELF object");
    let object_path = scratch.0.join("guest.o");
    let driver_path = scratch.0.join("driver.c");
    let executable_path = scratch.0.join("guest");
    fs::write(&object_path, object).expect("write native object");
    fs::write(
        &driver_path,
        "#include <stdint.h>\nextern int64_t native_value(void);\nint main(void) { return native_value() == INT64_C(42) ? 0 : 91; }\n",
    )
    .expect("write result oracle");
    let linked = Command::new("cc")
        .arg(&object_path)
        .arg(&driver_path)
        .arg("-o")
        .arg(&executable_path)
        .output()
        .expect("C toolchain required");
    assert!(
        linked.status.success(),
        "native link failed: {}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let run = Command::new(&executable_path)
        .output()
        .expect("execute native guest");
    assert_eq!(
        run.status.code(),
        Some(0),
        "native result mismatch: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}
