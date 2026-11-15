//! Candidate platform-core-v1 Java/LLVM/Wasm -> Rust Cranelift **native guest** gate.
//! The canonical fixture bytes in contracts/platform-core-v1/ are identical to
//! their Java/LLVM/Wasm peers (verify source Git blob SHAs separately).
//! Does NOT certify actors, platform ABI, stdlib, Wasm, or cross-backend parity.
//! Full-width output is observed through a generated C driver after changing
//! only the exported entrypoint name; exact original source is compiled too.
use oreslang_source_rs::{compile, compile_checked_i64};
use serde_json::Value;

const POSITIVE: [(&str, &str); 7] = [
    ("arith-precedence.ores", include_str!("../contracts/platform-core-v1/arith-precedence.ores")),
    ("signed-minimum.ores", include_str!("../contracts/platform-core-v1/signed-minimum.ores")),
    ("max-minus-five.ores", include_str!("../contracts/platform-core-v1/max-minus-five.ores")),
    ("two-negative-factors.ores", include_str!("../contracts/platform-core-v1/two-negative-factors.ores")),
    ("nested-unary.ores", include_str!("../contracts/platform-core-v1/nested-unary.ores")),
    ("negative-product.ores", include_str!("../contracts/platform-core-v1/negative-product.ores")),
    ("eager-checked-int.ores", include_str!("../contracts/platform-core-v1/eager-checked-int.ores")),
];
const NEGATIVE: [(&str, &str); 5] = [
    ("reject-unknown-symbol.ores", include_str!("../contracts/platform-core-v1/reject-unknown-symbol.ores")),
    ("reject-string-as-int.ores", include_str!("../contracts/platform-core-v1/reject-string-as-int.ores")),
    ("reject-overflow.ores", include_str!("../contracts/platform-core-v1/reject-overflow.ores")),
    ("reject-xor-operator.ores", include_str!("../contracts/platform-core-v1/reject-xor-operator.ores")),
    ("reject-no-value-return.ores", include_str!("../contracts/platform-core-v1/reject-no-value-return.ores")),
];

#[test]
fn canonical_manifest_has_exact_candidates_and_never_claims_backend_proof() {
    let manifest: Value =
        serde_json::from_str(include_str!("../contracts/platform-core-v1/manifest.json"))
            .expect("canonical fixture manifest must be valid JSON");
    assert_eq!(manifest["schema"], "oreslang.platform-core.v1");
    assert_eq!(manifest["positive"].as_array().unwrap().len(), POSITIVE.len());
    assert_eq!(manifest["negative"].as_array().unwrap().len(), NEGATIVE.len());
    assert!(manifest["scope"].as_str().unwrap().contains("no backend support is asserted"));
    for (cases, files) in [
        (manifest["positive"].as_array().unwrap(), POSITIVE.as_slice()),
        (manifest["negative"].as_array().unwrap(), NEGATIVE.as_slice()),
    ] {
        for (case, (file, source)) in cases.iter().zip(files) {
            assert_eq!(case["file"], *file, "ordered canonical fixture drift");
            assert!(source.starts_with("pub fnc main(): int"), "{file}: source changed");
        }
    }
}

#[test]
fn unimplemented_platform_language_constructs_remain_fail_closed() {
    // Rejection proves only safe admission, never runnable feature support.
    for source in [
        "define actor Worker as end",
        "define state Basket as end",
        "pub fnc main(): int { return trap get_value(); }",
        "pub fnc main(): int { return rt borrow value; }",
        "pub fnc main(): int { return await send(42); }",
        "pub fnc main(): int { return stdio.out.write(42); }",
        "pub fnc main(): int { return critical-read value; }",
    ] {
        assert!(compile(source).is_err(), "unsupported source emitted native object: {source}");
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod native {
    use super::{NEGATIVE, POSITIVE};
    use oreslang_source_rs::compile_checked_i64;
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::{Command, Output};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir()
                .join(format!("oreslang-platform-core-v1-{}-{stamp}", std::process::id()));
            fs::create_dir(&path).expect("create isolated native guest directory");
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run_renamed_entry(source: &str) -> Output {
        let before = "pub fnc main(): int";
        assert_eq!(source.matches(before).count(), 1, "fixture entrypoint drift");
        // The exact original fixture must compile before adapting the symbol
        // for a C observer capable of printing the full signed-i64 value.
        let original = compile_checked_i64(source).expect("exact canonical source rejected");
        assert!(original.starts_with(b"\x7fELF"), "must emit real ELF object");
        let renamed = source.replacen(before, "pub fnc platform_core_entry(): int", 1);
        let object_bytes = compile_checked_i64(&renamed).expect("renamed native entry rejected");
        let temp = Temp::new();
        let object = temp.0.join("guest.o");
        let c_source = temp.0.join("observe.c");
        let executable = temp.0.join("guest");
        fs::write(&object, object_bytes).unwrap();
        fs::write(
            &c_source,
            "#include <stdint.h>\n#include <inttypes.h>\n#include <stdio.h>\nextern int64_t platform_core_entry(void);\nint main(void) { printf(\"%\" PRId64 \"\\n\", platform_core_entry()); return 0; }\n",
        ).unwrap();
        let link = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg(&c_source).arg(&object).arg("-o").arg(&executable)
            .output().expect("system linker unavailable");
        assert!(link.status.success(), "link failed: {}", String::from_utf8_lossy(&link.stderr));
        Command::new(&executable).current_dir(&temp.0).output().expect("native guest failed to launch")
    }

    #[test]
    fn seven_canonical_positive_fixtures_execute_full_width_i64() {
        let manifest: serde_json::Value = serde_json::from_str(
            include_str!("../contracts/platform-core-v1/manifest.json")).unwrap();
        for (index, (name, source)) in POSITIVE.iter().enumerate() {
            let expected = manifest["positive"][index]["expected_stdout"].as_str().unwrap();
            let output = run_renamed_entry(source);
            assert!(output.status.success(), "{name}: guest: {:?}, stderr: {}",
                output.status, String::from_utf8_lossy(&output.stderr));
            assert_eq!(output.stdout, expected.as_bytes(), "{name}: full-width native result differs");
            assert!(output.stderr.is_empty(), "{name}: unexpected stderr");
        }
    }

    #[test]
    fn canonical_negative_fixtures_reject_or_trap_without_silent_wrap() {
        for (name, source) in NEGATIVE {
            if name == "reject-overflow.ores" {
                // Contract explicitly demands eager checked arithmetic during guest execution.
                // Unlike an ordinary wrapping Cranelift object, this is opt-in --checked-i64.
                let output = run_renamed_entry(source);
                assert_eq!(output.status.signal(), Some(4),
                    "{name}: expected native x86-64 SIGILL checked overflow trap, got {:?}", output.status);
                continue;
            }
            let result = compile_checked_i64(source);
            assert!(result.is_err(), "{name}: invalid guest emitted a native object");
        }
    }
}
