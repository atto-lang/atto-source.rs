//! Executed Rust/Cranelift parity checks against the Java + LLVM portable-i64
//! source contract. This is bounded scalar evidence, not checked-overflow parity.
use oreslang_source_rs::{compile_with_effect_ir, frontend};

#[test]
fn line_comment_block_markers_do_not_change_mir_or_effects() {
    let plain = "pub fnc main(): int { return 40 + 2; }";
    let with_comments = "// /* unterminated-looking, but it is a line comment\n\
         pub fnc main(): int { return 40 + 2; } // */ /* ignored";
    assert_eq!(
        frontend::parse(plain).unwrap(),
        frontend::parse(with_comments).unwrap()
    );
    let (object, effects) = compile_with_effect_ir(with_comments).unwrap();
    assert!(!object.is_empty());
    let (_, expected_effects) = compile_with_effect_ir(plain).unwrap();
    assert_eq!(effects, expected_effects);
}

#[cfg(target_os = "linux")]
mod native_i64 {
    use oreslang_source_rs::compile;
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
                .expect("system clock before epoch")
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "ores-rs-portable-i64-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&dir).expect("create native test directory");
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Link a real Cranelift object with cc and execute it. The optional C
    /// driver compares a full int64_t result instead of truncating to an
    /// eight-bit process exit status.
    fn execute_native(source: &str, driver: Option<&str>, expected_exit: i32) {
        let scratch = Scratch::new();
        let object_path = scratch.0.join("guest.o");
        let executable_path = scratch.0.join("guest");
        let bytes = compile(source).unwrap_or_else(|e| panic!("guest compile: {e}"));
        assert!(bytes.starts_with(b"\x7fELF"), "expected native ELF object");
        fs::write(&object_path, bytes).expect("write guest object");

        let mut command = Command::new("cc");
        command.arg(&object_path);
        if let Some(driver) = driver {
            let driver_path = scratch.0.join("driver.c");
            fs::write(&driver_path, driver).expect("write C i64 oracle");
            command.arg(&driver_path);
        }
        command.arg("-o").arg(&executable_path);
        let linked = command.output().expect("native C toolchain is required");
        assert!(
            linked.status.success(),
            "native link failed: {}",
            String::from_utf8_lossy(&linked.stderr)
        );

        let result = Command::new(&executable_path)
            .output()
            .expect("execute compiled native guest");
        assert_eq!(
            result.status.code(),
            Some(expected_exit),
            "native result mismatch for {source:?}: stderr={}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    fn execute_full_i64(expression: &str, expected: i64) {
        let source = format!("pub fnc parity_value(): int {{ return {expression}; }}");
        let c_expected = if expected == i64::MIN {
            "INT64_MIN".to_owned()
        } else {
            format!("INT64_C({expected})")
        };
        let driver = format!(
            "#include <stdint.h>\n\
             extern int64_t parity_value(void);\n\
             int main(void) {{ return parity_value() == {c_expected} ? 0 : 91; }}\n"
        );
        execute_native(&source, Some(&driver), 0);
    }

    #[test]
    fn portable_i64_contract_executes_complete_signed_results() {
        // Shared contracts/v1/source-i64/portable-i64.v1.json in Java/LLVM:
        // every intermediate fits i64; overflow/division are out of scope.
        for (expression, expected) in [
            ("2+3*4", 14),
            ("(2+3)*4", 20),
            ("(8-3)*5+2", 27),
            ("100-(3+2)-19", 76),
            ("3*4-2*5-7", -5),
            ("9223372036854775807-1", i64::MAX - 1),
            ("-9223372036854775808", i64::MIN),
            ("9007199254740993-9007199254740992", 1),
            ("9_007_199_254_740_993", 9_007_199_254_740_993),
        ] {
            execute_full_i64(expression, expected);
        }
    }

    #[test]
    fn shared_java_llvm_proxy_receipt_runs_in_rust_native_guest() {
        // Exact file content matches Java/LLVM canonical fixture blob
        // 13925491ac224bc5108097e73927968120eb182d.
        execute_native(include_str!("fixtures/proxy-packed-receipt.ores"), None, 42);
    }
}
