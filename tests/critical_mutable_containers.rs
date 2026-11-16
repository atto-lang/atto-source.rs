//! Ten *additional* scalar mutable-container scenarios supplement the
//! original ten-case critical keyword matrix. Native Rust and native/Wasm
//! corpus runners consume these fixtures with exact diagnostics/stdout.
use std::fs;
use std::path::Path;

const CASES: [(&str, bool, &str); 10] = [
    ("pass/214-state-multi-field-exclusive.atto", true, ""),
    ("pass/215-state-two-field-snapshot.atto", true, ""),
    ("pass/216-state-multi-field-instance-isolation.atto", true, ""),
    ("pass/217-state-nominal-container-ownership.atto", true, ""),
    ("pass/218-state-repeated-writes-one-lease.atto", true, ""),
    ("fail/216-state-string-reference-disallowed.atto", false, "E_UNSUPPORTED_TYPE"),
    ("fail/217-state-multi-field-duplicate.atto", false, "E_DUPLICATE"),
    ("fail/218-state-scalar-type-confusion.atto", false, "E_TYPE"),
    ("fail/219-state-same-type-other-instance.atto", false, "E_STATE_BORROW"),
    ("fail/220-state-write-effect-escape.atto", false, "E_STATE_BORROW"),
];

#[test]
fn ten_more_mutable_state_container_scenarios() {
    for (path, accepts, want) in CASES {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures").join(path);
        let source = fs::read_to_string(&fixture)
            .unwrap_or_else(|e| panic!("{}: {e}", fixture.display()));
        let result = atto_compiler::compile(&source);
        if accepts {
            let wasm = result.unwrap_or_else(|d| panic!("{path}: {d}"));
            assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{path}: not Wasm");
            assert_eq!(atto_compiler::compile(&source).unwrap(), wasm,
                "{path}: compilation must be deterministic");
        } else {
            let err = result.expect_err(&format!("{path}: unsafe artifact emitted"));
            assert_eq!(err.code, want, "{path}: {err}");
            assert_eq!(atto_compiler::compile(&source).unwrap_err().code, want);
        }
    }
}

/// Future collection-owned state cannot be silently admitted by the scalar
/// frontend; once supported, this test must be replaced with real deep-loan
/// fixtures and target-specific guest execution.
#[test]
fn mutable_reference_collections_are_explicitly_unsupported() {
    for ty in ["Array<int>", "Map<string,int>", "Set<int>", "Pointer<int>"] {
        let source = format!(
            "define state container Unsafe as\n state {{ let {ty} items: 0; }}\nend\npub fnc main(): int {{ return 0; }}");
        assert!(atto_compiler::compile(&source).is_err(),
            "{ty}: compiler admitted unproven mutable reference ownership");
    }
}
