//! Direct Rust frontend/MIR/Cranelift tests of canonical Java+LLVM scalar
//! source files. No Java/LLVM guest execution or actor/runtime parity is implied.
use oreslang_source_rs::{compile, frontend, mir};

fn positives() -> [(&'static str, &'static str, i64); 10] {
    [
        (
            "canonical-add",
            include_str!("../contracts/atto-scalar-v1/positive/canonical-add.atto"),
            42,
        ),
        (
            "precedence",
            include_str!("../contracts/atto-scalar-v1/positive/precedence.atto"),
            14,
        ),
        (
            "parentheses",
            include_str!("../contracts/atto-scalar-v1/positive/parentheses.atto"),
            20,
        ),
        (
            "signed-minimum",
            include_str!("../contracts/atto-scalar-v1/positive/signed-minimum.atto"),
            i64::MIN,
        ),
        (
            "signed-max-minus-one",
            include_str!("../contracts/atto-scalar-v1/positive/signed-max-minus-one.atto"),
            i64::MAX - 1,
        ),
        (
            "type-first-readonly",
            include_str!("../contracts/atto-scalar-v1/positive/type-first-readonly.atto"),
            42,
        ),
        (
            "typed-const-readonly",
            include_str!("../contracts/atto-scalar-v1/positive/typed-const-readonly.atto"),
            42,
        ),
        (
            "inferred-const-readonly",
            include_str!("../contracts/atto-scalar-v1/positive/inferred-const-readonly.atto"),
            42,
        ),
        (
            "pure-call",
            include_str!("../contracts/atto-scalar-v1/positive/pure-call.atto"),
            42,
        ),
        (
            "type-first-parameter",
            include_str!("../contracts/atto-scalar-v1/positive/type-first-parameter.atto"),
            42,
        ),
    ]
}

fn negatives() -> [(&'static str, &'static str, &'static str); 5] {
    [
        (
            "type-first-reassignment",
            include_str!("../contracts/atto-scalar-v1/negative/type-first-reassignment.atto"),
            "immutable_binding",
        ),
        (
            "typed-const-reassignment",
            include_str!("../contracts/atto-scalar-v1/negative/typed-const-reassignment.atto"),
            "immutable_binding",
        ),
        (
            "removed-modulo",
            include_str!("../contracts/atto-scalar-v1/negative/removed-modulo.atto"),
            "removed_operator",
        ),
        (
            "removed-caret",
            include_str!("../contracts/atto-scalar-v1/negative/removed-caret.atto"),
            "removed_operator",
        ),
        (
            "removed-double-caret",
            include_str!("../contracts/atto-scalar-v1/negative/removed-double-caret.atto"),
            "removed_operator",
        ),
    ]
}

#[test]
fn canonical_sources_keep_exact_java_llvm_git_blob_ids() {
    // This also protects copied canonical source bytes from future drift.
    // Git is already required by the source-owned exact-checkout CI gate.
    for (path, expected) in [
        ("positive/canonical-add.atto", "015f8b0f79aafa796a148d2adeff1574e8d6c491"),
        ("positive/precedence.atto", "50414e5e95c8007f066696f55aa75309555b83e2"),
        ("positive/parentheses.atto", "9f508169c741486d5cec7510ac84fb761f342def"),
        ("positive/signed-minimum.atto", "450ebee951733d3f0b567413e70724155d26ce13"),
        ("positive/signed-max-minus-one.atto", "c400f440fde2fbc10514ab7cfd2ffe3d19729ee1"),
        ("positive/type-first-readonly.atto", "75ff4afc98fc65a774611eb7686e893f3d28a6da"),
        ("positive/typed-const-readonly.atto", "0325018c54dd3534ddac76dc29724aa61e979683"),
        ("positive/inferred-const-readonly.atto", "4fe471862d90ba57e135b93728bd44690ff2c894"),
        ("positive/pure-call.atto", "e5ab32f7d8f684d655a83a3df989f16c52090c05"),
        ("positive/type-first-parameter.atto", "3d6c16ed04aa86f954461885ec77c8f10b47fb5c"),
        ("negative/type-first-reassignment.atto", "b118f53e68565dcb9e6acd58deb8ee47dca67606"),
        ("negative/typed-const-reassignment.atto", "784ebd47a50211e6f00cb732cc74319ed1e58fac"),
        ("negative/removed-modulo.atto", "45d5794870e93074d9b11b90833a849e96aaa4c1"),
        ("negative/removed-caret.atto", "6889c7e83c8d94b906fbfe78723caee9580da6aa"),
        ("negative/removed-double-caret.atto", "7ee99948018e927cce74c9ad0edc1975e4251689"),
    ] {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/atto-scalar-v1")
            .join(path);
        let hash = std::process::Command::new("git")
            .arg("hash-object")
            .arg("--")
            .arg(&file)
            .output()
            .expect("git hash-object required for canonical source identity");
        assert!(
            hash.status.success(),
            "{path}: hash-object failed: {}",
            String::from_utf8_lossy(&hash.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&hash.stdout).trim(),
            expected,
            "{path}: canonical Java/LLVM source changed"
        );
    }
}

#[test]
fn canonical_positive_sources_reach_native_codegen() {
    for (name, source, _) in positives() {
        let parsed = frontend::parse(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lowered = mir::lower(parsed).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!lowered.functions.is_empty(), "{name}");
        let object = compile(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!object.is_empty(), "{name}: native object absent");
    }
}

#[test]
fn canonical_negatives_are_rejected_before_native_emission() {
    for (name, source, diagnostic) in negatives() {
        let error = compile(source).expect_err("invalid guest compiled");
        assert!(error.starts_with(diagnostic), "{name}: {error}");
    }
}

#[test]
fn canonical_parameter_spellings_are_ast_equivalent() {
    let prefix = "pub fnc add(int x, int y): int { return x+y; } pub fnc main(): int { return add(20,22); }";
    let suffix = "pub fnc add(x: int, y: int): int { return x+y; } pub fnc main(): int { return add(20,22); }";
    let mixed = "pub fnc add(int x, y: int): int { return x+y; } pub fnc main(): int { return add(20,22); }";
    let expected = frontend::parse(prefix).expect("type-first baseline");
    for source in [suffix, mixed] {
        let parsed = frontend::parse(source).expect("alternate typed formal syntax");
        assert_eq!(parsed, expected);
        mir::lower(parsed).expect("identical scalar MIR semantics");
        assert!(!compile(source).expect("Cranelift object").is_empty());
    }
}

#[test]
fn typed_parameters_are_never_inferred_or_defaulted() {
    for source in [
        "pub fnc f(x): int { return x; }",
        "pub fnc f(x: bool): int { return 42; }",
        "pub fnc f(x:): int { return 42; }",
        "pub fnc f(x: int = 7): int { return x; }",
        "pub fnc f(int x, y): int { return x; }",
        "pub fnc f(: int): int { return 42; }",
        "pub fnc f(x: int,): int { return x; }",
        "pub fnc f(x: any): int { return 42; }",
        "pub fnc f(x: unknown): int { return 42; }",
        "pub fnc f(x: float): int { return 42; }",
        "pub fnc f(x: int?): int { return 42; }",
        "pub fnc f(x: Option<int>): int { return 42; }",
        "pub fnc f(x: int, x): int { return x; }",
    ] {
        let error = compile(source).expect_err("unsupported parameter admitted");
        assert!(error.starts_with("syntax_error"), "{source}: {error}");
    }
}

#[test]
fn mixed_parameter_spellings_keep_duplicate_and_arity_checks() {
    for source in [
        "pub fnc f(x: int, int x): int { return x; }",
        "pub fnc f(int x, x: int): int { return x; }",
        "pub fnc f(x: int, x: int): int { return x; }",
        "pub fnc f(x: int): int { int x = 7; return x; }",
    ] {
        let error = compile(source).expect_err("duplicate admitted");
        assert!(error.starts_with("duplicate_declaration"), "{source}: {error}");
    }
    for source in [
        "pub fnc f(x: int): int { return x; } pub fnc main(): int { return f(); }",
        "pub fnc f(x: int): int { return x; } pub fnc main(): int { return f(1,2); }",
        "pub fnc f(x: int, int y): int { return x+y; } pub fnc main(): int { return f(42); }",
    ] {
        let error = compile(source).expect_err("wrong arity admitted");
        assert!(error.starts_with("call_arity"), "{source}: {error}");
    }
}

#[test]
fn removed_operators_have_canonical_diagnostics_and_exact_offsets() {
    for (symbol, offset) in [("%", 31), ("^", 31), ("^^", 31)] {
        let prefix = "pub fnc main(): int { return 1 ";
        assert_eq!(prefix.len(), offset);
        let source = format!("{prefix}{symbol} 2; }}");
        let err = frontend::parse(&source).expect_err("removed operator admitted");
        assert_eq!(err.class, "removed_operator");
        assert_eq!(err.offset, offset);
        assert!(
            compile(&source).unwrap_err().starts_with("removed_operator"),
            "{source}"
        );
    }
}

#[test]
fn removed_operators_inside_comments_are_not_guest_tokens() {
    for source in [
        "/* % ^ ^^ */ pub fnc main(): int { return 42; }",
        "// % ^ ^^\npub fnc main(): int { return 42; }",
        "/* outer /* % ^^ */ ^ */ pub fnc main(): int { return 42; }",
    ] {
        assert!(!compile(source).expect("comment was mislexed").is_empty());
    }
}

#[test]
fn adversarial_unary_and_left_associative_chains_are_bounded() {
    for expression in [
        format!("{}1", "-".repeat(frontend::MAX_PRIMARY_DEPTH + 1)),
        "1+".repeat(frontend::MAX_INFIX_OPERATORS + 1) + "1",
        "1*".repeat(frontend::MAX_INFIX_OPERATORS + 1) + "1",
    ] {
        let source = format!("pub fnc main(): int {{ return {expression}; }}");
        let error = frontend::parse(&source).expect_err("expression budget bypass");
        assert_eq!(error.class, "resource_limit", "{source}: {error}");
        assert!(compile(&source).unwrap_err().starts_with("resource_limit"));
    }
}

#[cfg(target_os = "linux")]
mod linux_native {
    use super::{negatives, positives};
    use std::{
        fs,
        path::PathBuf,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rs-shared-scalar-v1-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn strict_aot_compiles_links_and_executes_all_ten_java_llvm_guests() {
        let dir = Scratch::new();
        for (name, source, expected) in positives() {
            let input = dir.0.join(format!("{name}.atto"));
            let object = dir.0.join(format!("{name}.o"));
            let executable = dir.0.join(format!("{name}.exe"));
            fs::write(&input, source).expect("exact canonical guest source");
            let compiled = Command::new(env!("CARGO_BIN_EXE_oresc"))
                .arg("--strict-aot")
                .arg(&input)
                .arg(&object)
                .output()
                .expect("run actual Rust compiler");
            assert!(
                compiled.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let bytes = fs::read(&object).expect("native object");
            assert!(bytes.starts_with(b"\x7fELF"), "{name}: not an ELF object");
            let linked = Command::new("cc")
                .arg(&object)
                .arg("-o")
                .arg(&executable)
                .output()
                .expect("C linker required");
            assert!(
                linked.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&linked.stderr)
            );
            let guest = Command::new(&executable)
                .output()
                .expect("run native AttoLang guest");
            assert_eq!(
                guest.status.code(),
                Some((expected as u8) as i32),
                "{name}: exit code mismatch"
            );
            assert!(guest.stdout.is_empty(), "{name}: unexpected stdout");
            assert!(guest.stderr.is_empty(), "{name}: unexpected stderr");
        }
    }

    #[test]
    fn full_width_i64_extremes_survive_actual_native_abi() {
        let dir = Scratch::new();
        for (name, source, oracle) in [
            ("signed-minimum", positives()[3].1, "INT64_MIN"),
            ("signed-max-minus-one", positives()[4].1, "(INT64_MAX - 1)"),
        ] {
            // Source is identical to canonical except the exported ABI name.
            let renamed = source.replace("pub fnc main()", "pub fnc parity_value()");
            assert_ne!(renamed, source);
            let object = dir.0.join(format!("{name}.o"));
            fs::write(
                &object,
                oreslang_source_rs::compile(&renamed).expect("Cranelift object"),
            )
            .expect("write native object");
            let driver = dir.0.join(format!("{name}.c"));
            fs::write(
                &driver,
                format!(
                    "#include <stdint.h>\nextern int64_t parity_value(void);\nint main(void) {{ return parity_value() == {oracle} ? 0 : 91; }}\n"
                ),
            )
            .expect("write full-width C oracle");
            let executable = dir.0.join(format!("{name}.exe"));
            let linked = Command::new("cc")
                .arg(&object)
                .arg(&driver)
                .arg("-o")
                .arg(&executable)
                .output()
                .expect("link C i64 oracle");
            assert!(
                linked.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&linked.stderr)
            );
            assert_eq!(
                Command::new(&executable).status().unwrap().code(),
                Some(0),
                "{name}: i64 ABI truncated"
            );
        }
    }

    #[test]
    fn mixed_parameter_order_and_high_i64_bits_survive_native_abi() {
        let dir = Scratch::new();
        let object = dir.0.join("mixed.o");
        let source = "pub fnc mixed(a: int, int b, c: int, int d): int { return a-b+c-d; }";
        fs::write(
            &object,
            oreslang_source_rs::compile(source).expect("mixed-param native object"),
        )
        .expect("write object");
        let driver = dir.0.join("mixed.c");
        fs::write(
            &driver,
            "#include <stdint.h>\nextern int64_t mixed(int64_t,int64_t,int64_t,int64_t);\nint main(void) { return mixed(INT64_C(9007199254740993),INT64_C(9007199254740992),42,1) == 42 ? 0 : 91; }\n",
        )
        .expect("write C ABI probe");
        let executable = dir.0.join("mixed.exe");
        let linked = Command::new("cc")
            .arg(&object)
            .arg(&driver)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("native linker");
        assert!(
            linked.status.success(),
            "{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        assert_eq!(
            Command::new(&executable).status().unwrap().code(),
            Some(0),
            "mixed parameter order or signed i64 precision corrupted"
        );
    }

    #[test]
    fn invalid_sources_publish_no_object_and_cannot_overwrite_existing_object() {
        let dir = Scratch::new();
        for (name, source, class) in negatives() {
            let input = dir.0.join(format!("{name}.atto"));
            let absent = dir.0.join(format!("{name}.o"));
            let sentinel = dir.0.join(format!("{name}.existing.o"));
            fs::write(&input, source).expect("write negative");
            fs::write(&sentinel, b"PREEXISTING").expect("write sentinel");
            for output in [&absent, &sentinel] {
                let compiled = Command::new(env!("CARGO_BIN_EXE_oresc"))
                    .arg("--strict-aot")
                    .arg(&input)
                    .arg(output)
                    .output()
                    .expect("run invalid source through real compiler");
                assert!(!compiled.status.success(), "{name}: invalid guest accepted");
                assert!(compiled.stdout.is_empty(), "{name}: misleading stdout");
                assert!(
                    String::from_utf8_lossy(&compiled.stderr).contains(class),
                    "{name}: {}",
                    String::from_utf8_lossy(&compiled.stderr)
                );
            }
            assert!(!absent.exists(), "{name}: object published for invalid source");
            assert_eq!(
                fs::read(&sentinel).unwrap(),
                b"PREEXISTING",
                "{name}: existing artifact overwritten"
            );
        }
    }
}
