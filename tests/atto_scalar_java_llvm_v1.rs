//! Actual Rust frontend/MIR/Cranelift admission of Java and LLVM scalar-v1
//! fixtures. A passing Rust build is not a five-backend parity certificate.
use oreslang_source_rs::{compile, frontend, mir};

fn positives() -> [(&'static str, &'static str, i64); 10] {
    [
        ("canonical-add", include_str!("../contracts/atto-scalar-v1/positive/canonical-add.atto"), 42),
        ("precedence", include_str!("../contracts/atto-scalar-v1/positive/precedence.atto"), 14),
        ("parentheses", include_str!("../contracts/atto-scalar-v1/positive/parentheses.atto"), 20),
        ("signed-minimum", include_str!("../contracts/atto-scalar-v1/positive/signed-minimum.atto"), i64::MIN),
        ("signed-max-minus-one", include_str!("../contracts/atto-scalar-v1/positive/signed-max-minus-one.atto"), i64::MAX - 1),
        ("type-first-readonly", include_str!("../contracts/atto-scalar-v1/positive/type-first-readonly.atto"), 42),
        ("typed-const-readonly", include_str!("../contracts/atto-scalar-v1/positive/typed-const-readonly.atto"), 42),
        ("inferred-const-readonly", include_str!("../contracts/atto-scalar-v1/positive/inferred-const-readonly.atto"), 42),
        ("pure-call", include_str!("../contracts/atto-scalar-v1/positive/pure-call.atto"), 42),
        ("type-first-parameter", include_str!("../contracts/atto-scalar-v1/positive/type-first-parameter.atto"), 42),
    ]
}
fn negatives() -> [(&'static str, &'static str, &'static str); 5] {
    [
        ("type-first-reassignment", include_str!("../contracts/atto-scalar-v1/negative/type-first-reassignment.atto"), "immutable_binding"),
        ("typed-const-reassignment", include_str!("../contracts/atto-scalar-v1/negative/typed-const-reassignment.atto"), "immutable_binding"),
        ("removed-modulo", include_str!("../contracts/atto-scalar-v1/negative/removed-modulo.atto"), "syntax_error"),
        ("removed-caret", include_str!("../contracts/atto-scalar-v1/negative/removed-caret.atto"), "syntax_error"),
        ("removed-double-caret", include_str!("../contracts/atto-scalar-v1/negative/removed-double-caret.atto"), "syntax_error"),
    ]
}

#[test]
fn canonical_positive_sources_reach_native_codegen() {
    for (name, source, _) in positives() {
        let parsed = frontend::parse(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lowered = mir::lower(parsed).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!lowered.functions.is_empty(), "{name}");
        let object = compile(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!object.is_empty(), "{name}: missing native object");
    }
}

#[test]
fn canonical_negatives_are_rejected_before_native_object_emission() {
    for (name, source, diagnostic) in negatives() {
        let error = compile(source).expect_err("negative compiled successfully");
        assert!(error.starts_with(diagnostic), "{name}: {error}");
    }
}

#[test]
fn canonical_parameter_spellings_are_ast_equivalent() {
    let prefix = "pub fnc add(int x, int y): int { return x+y; } pub fnc main(): int { return add(20,22); }";
    let suffix = "pub fnc add(x: int, y: int): int { return x+y; } pub fnc main(): int { return add(20,22); }";
    let mixed = "pub fnc add(int x, y: int): int { return x+y; } pub fnc main(): int { return add(20,22); }";
    let expected = frontend::parse(prefix).unwrap();
    for source in [suffix, mixed] {
        let parsed = frontend::parse(source).unwrap();
        assert_eq!(parsed, expected);
        mir::lower(parsed).expect("typed MIR");
        assert!(!compile(source).unwrap().is_empty());
    }
}

#[test]
fn name_first_parameters_are_typed_and_not_inferred() {
    for source in [
        "pub fnc f(x): int { return x; }",
        "pub fnc f(x: bool): int { return 42; }",
        "pub fnc f(x:): int { return 42; }",
        "pub fnc f(x: int = 7): int { return x; }",
        "pub fnc f(int x, y): int { return x; }",
        "pub fnc f(: int): int { return 42; }",
        "pub fnc f(x: int,): int { return x; }",
    ] {
        let error = compile(source).expect_err("malformed typed parameter admitted");
        assert!(error.starts_with("syntax_error"), "{source}: {error}");
    }
}

#[test]
fn mixed_parameter_spellings_keep_duplicate_and_arity_checks() {
    for source in [
        "pub fnc f(x: int, int x): int { return x; }",
        "pub fnc f(int x, x: int): int { return x; }",
        "pub fnc f(x: int): int { int x = 7; return x; }",
    ] {
        let error = compile(source).expect_err("duplicate scalar admitted");
        assert!(error.starts_with("duplicate_declaration"), "{source}: {error}");
    }
    for source in [
        "pub fnc f(x: int): int { return x; } pub fnc main(): int { return f(); }",
        "pub fnc f(x: int): int { return x; } pub fnc main(): int { return f(1,2); }",
    ] {
        let error = compile(source).expect_err("wrong call arity admitted");
        assert!(error.starts_with("call_arity"), "{source}: {error}");
    }
}

#[cfg(target_os = "linux")]
mod linux_native {
    use super::{negatives, positives};
    use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH)
                .expect("system clock").as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rs-shared-scalar-v1-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).expect("scratch dir");
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn strict_aot_compiles_links_and_executes_all_ten_java_llvm_guests() {
        let dir = Scratch::new();
        for (name, source, expected) in positives() {
            let input = dir.0.join(format!("{name}.atto"));
            let obj = dir.0.join(format!("{name}.o"));
            let exe = dir.0.join(format!("{name}.exe"));
            fs::write(&input, source).expect("exact guest source");
            let run = Command::new(env!("CARGO_BIN_EXE_oresc"))
                .arg("--strict-aot").arg(&input).arg(&obj).output().expect("oresc");
            assert!(run.status.success(), "{name}: {}", String::from_utf8_lossy(&run.stderr));
            let object = fs::read(&obj).expect("native object");
            assert!(object.starts_with(b"\x7fELF"), "{name}: not ELF");
            let link = Command::new("cc").arg(&obj).arg("-o").arg(&exe)
                .output().expect("cc");
            assert!(link.status.success(), "{name}: {}", String::from_utf8_lossy(&link.stderr));
            let guest = Command::new(&exe).output().expect("run actual compiled guest");
            assert_eq!(guest.status.code(), Some((expected as u8) as i32), "{name}: exit mismatch");
            assert!(guest.stdout.is_empty(), "{name}: unexpected stdout");
            assert!(guest.stderr.is_empty(), "{name}: unexpected stderr");
        }
    }

    #[test]
    fn full_width_i64_boundary_values_survive_native_abi() {
        let dir = Scratch::new();
        for (name, source, oracle) in [
            ("signed-minimum", positives()[3].1, "INT64_MIN"),
            ("signed-max-minus-one", positives()[4].1, "(INT64_MAX - 1)"),
        ] {
            // The exact canonical main() was already linked and run above.
            // Rename only its exported symbol for a full-width C ABI oracle.
            let wrapped = source.replace("pub fnc main()", "pub fnc parity_value()");
            assert_ne!(wrapped, source);
            let obj = dir.0.join(format!("{name}.o"));
            fs::write(&obj, oreslang_source_rs::compile(&wrapped).expect("guest object")).unwrap();
            let c = dir.0.join(format!("{name}.c"));
            fs::write(&c, format!("#include <stdint.h>\nextern int64_t parity_value(void);\nint main(void) {{ return parity_value() == {oracle} ? 0 : 91; }}\n")).unwrap();
            let exe = dir.0.join(format!("{name}.exe"));
            let link = Command::new("cc").arg(&obj).arg(&c).arg("-o").arg(&exe)
                .output().expect("cc native i64 oracle");
            assert!(link.status.success(), "{name}: {}", String::from_utf8_lossy(&link.stderr));
            assert_eq!(Command::new(&exe).status().unwrap().code(), Some(0), "{name}: truncated ABI");
        }
    }

    #[test]
    fn negative_sources_publish_no_object_and_cannot_overwrite_existing_object() {
        let dir = Scratch::new();
        for (name, source, class) in negatives() {
            let input = dir.0.join(format!("{name}.atto"));
            let absent = dir.0.join(format!("{name}.o"));
            let sentinel = dir.0.join(format!("{name}.existing.o"));
            fs::write(&input, source).unwrap();
            fs::write(&sentinel, b"PREEXISTING").unwrap();
            for output in [&absent, &sentinel] {
                let run = Command::new(env!("CARGO_BIN_EXE_oresc"))
                    .arg("--strict-aot").arg(&input).arg(output).output().expect("oresc");
                assert!(!run.status.success(), "{name}: bad guest compiled");
                assert!(run.stdout.is_empty(), "{name}: misleading stdout");
                assert!(String::from_utf8_lossy(&run.stderr).contains(class),
                    "{name}: {}", String::from_utf8_lossy(&run.stderr));
            }
            assert!(!absent.exists(), "{name}: invalid native object published");
            assert_eq!(fs::read(&sentinel).unwrap(), b"PREEXISTING", "{name}: artifact replaced");
        }
    }
}
