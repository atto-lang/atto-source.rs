//! Exactly ten auditable compile-level scenarios; the existing Node/Wasm
//! conformance harness also executes pass fixtures as genuine guest modules.
use std::fs;
use std::path::Path;

const CASES: [(&str, bool, &str); 10] = [
    ("pass/210-critical-alias-int.atto", true, ""),
    ("pass/211-critical-write-bool.atto", true, ""),
    ("pass/212-critical-read-snapshot.atto", true, ""),
    ("pass/213-critical-two-containers.atto", true, ""),
    ("fail/210-critical-read-block-escalation.atto", false, "E_STATE_BORROW"),
    ("fail/211-critical-read-mutation.atto", false, "E_STATE_BORROW"),
    ("fail/212-critical-unguarded-self.atto", false, "E_STATE_ACCESS"),
    ("fail/213-critical-binder-escape.atto", false, "E_ESCAPE"),
    ("fail/214-critical-spaced-keyword.atto", false, "E_SYNTAX"),
    ("fail/215-critical-nested-loan.atto", false, "E_STATE_BORROW"),
];

#[test]
fn ten_critical_scenarios_fail_closed_and_compile_deterministically() {
    for (path, pass, diagnostic) in CASES {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures").join(path);
        let source = fs::read_to_string(&fixture)
            .unwrap_or_else(|e| panic!("{}: {e}", fixture.display()));
        let first = atto_compiler::compile(&source);
        if pass {
            let bytes = first.unwrap_or_else(|e| panic!("{path}: unexpected {e}"));
            assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "{path}: missing Wasm header");
            assert_eq!(atto_compiler::compile(&source).unwrap(), bytes,
                "{path}: non-deterministic Wasm output");
        } else {
            let err = first.expect_err(&format!("{path}: unauthorized guest artifact emitted"));
            assert_eq!(err.code, diagnostic, "{path}: {err}");
            assert_eq!(atto_compiler::compile(&source).unwrap_err().code,
                diagnostic, "{path}: diagnostics not stable");
        }
    }
}
