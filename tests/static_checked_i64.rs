//! Statically provable signed-i64 overflow: bounded Rust MIR admission.
//! The actual dynamic Cranelift arithmetic path is NOT checked/certified here.
use oreslang_source_rs::{compile, compile_with_effect_ir, frontend, mir};

fn rejects_checked_overflow(source: &str) {
    let ast = frontend::parse(source).expect("checked arithmetic source parses");
    let err = mir::lower(ast).expect_err("statically proven overflow must fail MIR admission");
    assert_eq!(err.class, "checked_overflow", "{source}");
    let compile_error = compile(source).expect_err("overflow must not emit an object");
    assert!(
        compile_error.starts_with("checked_overflow:"),
        "{source}: {compile_error}"
    );
    let effect_error = compile_with_effect_ir(source)
        .expect_err("overflow must not emit paired guest object or effect facts");
    assert!(
        effect_error.starts_with("checked_overflow:"),
        "{source}: {effect_error}"
    );
}

#[test]
fn canonical_java_checked_overflow_sources_fail_closed_before_codegen() {
    for (name, source) in [
        (
            "checked-add-overflow",
            include_str!("fixtures/atto_scalar_checked_v1/checked-add-overflow.atto"),
        ),
        (
            "checked-sub-overflow",
            include_str!("fixtures/atto_scalar_checked_v1/checked-sub-overflow.atto"),
        ),
        (
            "checked-mul-overflow",
            include_str!("fixtures/atto_scalar_checked_v1/checked-mul-overflow.atto"),
        ),
        (
            "eager-unused-overflow",
            include_str!("fixtures/atto_scalar_checked_v1/eager-unused-overflow.atto"),
        ),
    ] {
        rejects_checked_overflow(source);
        assert!(source.ends_with('\n'), "{name}: must preserve canonical source bytes");
    }
}

#[test]
fn constant_aliases_and_explicit_mutable_ssa_versions_are_checked() {
    for body in [
        "const x = 9223372036854775807; return x + 1;",
        "val x: int = -9223372036854775808; return x - 1;",
        "let x = 9223372036854775807; x += 1; return x;",
        "mut int x = -9223372036854775808; x -= 1; return x;",
        "let x = 3037000500; x = x * x; return x;",
        "const x = -9223372036854775808; return -x;",
        "let x = 9223372036854775807; x -= 2; x += 3; return x;",
    ] {
        rejects_checked_overflow(&format!("pub fnc main(): int {{ {body} }}"));
    }
}

#[test]
fn nested_overflow_is_not_hidden_by_zero_or_runtime_dependent_parent() {
    for expression in [
        "(9223372036854775807 + 1) * 0",
        "0 - (9223372036854775807 + 1)",
        "(9223372036854775807 + 1) + input(1)",
        "input(1) + (9223372036854775807 + 1)",
        "3037000500 * 3037000500 * 0",
    ] {
        let source = format!(
            "pub fnc main(): int {{ return {expression}; }} fnc input(int n): int {{ return n; }}"
        );
        rejects_checked_overflow(&source);
    }
}

#[test]
fn eager_inactive_local_and_call_argument_are_validated() {
    for body in [
        "const ignored = 9223372036854775807 + 1; return 42;",
        "let unused = 3037000500 * 3037000500; return 42;",
        "return ignore(9223372036854775807 + 1);",
    ] {
        let source = format!(
            "pub fnc main(): int {{ {body} }} fnc ignore(int n): int {{ return 42; }}"
        );
        rejects_checked_overflow(&source);
    }
}

#[test]
fn safe_signed_boundaries_and_dynamic_inputs_remain_admitted() {
    for source in [
        "pub fnc main(): int { return 9223372036854775807 - 1; }",
        "pub fnc main(): int { return -9223372036854775808 + 1; }",
        "pub fnc main(): int { return (-7) * (-6); }",
        "pub fnc main(): int { let x = 40; x += 1; return x + 1; }",
        "pub fnc main(): int { let x = 9007199254740992; x += 1; return x; }",
        "pub fnc main(): int { return inc(41); } fnc inc(int x): int { return x + 1; }",
    ] {
        assert!(compile(source).is_ok(), "safe or dynamic source: {source}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn safe_rebinding_compiles_links_and_executes_real_cranelift_guest() {
    use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "ores-rs-checked-constant-native-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir(&dir).expect("create isolated native guest directory");
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let source = concat!(
        "pub fnc main(): int { let x = 9223372036854775807; ",
        "x -= 9223372036854775765; return x; }"
    );
    let bytes = compile(source).expect("checked safe path must emit native object");
    assert!(bytes.starts_with(b"\x7fELF"), "expected real ELF object");
    let object = dir.join("guest.o");
    let guest = dir.join("guest");
    fs::write(&object, bytes).expect("write native object");
    let link = Command::new("cc").arg(&object).arg("-o").arg(&guest).output()
        .expect("native C linker required");
    assert!(link.status.success(), "link: {}", String::from_utf8_lossy(&link.stderr));
    let result = Command::new(&guest).output().expect("execute checked-safe native guest");
    assert_eq!(result.status.code(), Some(42), "{:?}", result.stderr);
}
