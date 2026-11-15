use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_compiler(source: &str, extension: &str, object_mode: bool) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "oresc-structural-depth-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("create isolated compiler fixture directory");
    let input = dir.join(format!("guest.{extension}"));
    let output = dir.join("guest.o");
    std::fs::write(&input, source).expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_oresc"));
    if object_mode {
        command.arg(&input).arg(&output);
    } else {
        command.arg("--check").arg(&input);
    }
    let result = command.output().expect("execute actual AOT compiler");
    if object_mode {
        assert_eq!(
            output.exists(),
            result.status.success(),
            "object publication must match compiler admission"
        );
    }
    std::fs::remove_dir_all(&dir).expect("remove isolated fixture directory");
    result
}

fn source_for(terms: usize) -> String {
    format!(
        "pub fnc main(): int {{ return {}; }}",
        vec!["1"; terms].join("+")
    )
}

#[test]
fn structurally_bounded_scalar_chains_admit_check_and_object_emission() {
    let source = source_for(24);
    for extension in ["atto", "ores"] {
        for object_mode in [false, true] {
            let output = run_compiler(&source, extension, object_mode);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{extension}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
        }
    }
}

#[test]
fn long_left_spines_fail_closed_before_aot_artifact_or_check_output() {
    for source in [
        source_for(130),
        format!(
            "pub fnc main(): int {{ const x: int = {}; return x; }}",
            vec!["1"; 130].join("+")
        ),
        format!(
            "pub fnc main(): int {{ return helper({}); }} fnc helper(int x): int {{ return x; }}",
            vec!["1"; 130].join("+")
        ),
    ] {
        for extension in ["atto", "ores"] {
            for object_mode in [false, true] {
                let output = run_compiler(&source, extension, object_mode);
                assert_eq!(output.status.code(), Some(1));
                assert!(output.stdout.is_empty(), "rejection emitted success output");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stderr.contains("resource_limit") && stderr.contains("expression AST depth"),
                    "unexpected rejection diagnostic: {stderr}"
                );
            }
        }
    }
}
