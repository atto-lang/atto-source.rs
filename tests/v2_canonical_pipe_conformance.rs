//! Byte-identical pinned source snapshot of the canonical directed-pipe
//! conformance fixtures from atto-lang/atto-platform-conformance PR #102.
//! These tests compile *actual Atto guest source* through the public compiler.
//! A compile PASS alone does not prove a guest graph executes successfully.

const FORWARD: &str = include_str!("fixtures/conformance/pipe-rpipe-v1/actor-forward.atto");
const REVERSE: &str = include_str!("fixtures/conformance/pipe-rpipe-v1/actor-reverse.atto");
const MISSING_EMITS: &str = include_str!("fixtures/conformance/pipe-rpipe-v1/actor-missing-emits.atto");
const WRONG_DIRECTION: &str = include_str!("fixtures/conformance/pipe-rpipe-v1/actor-wrong-direction.atto");
const MIXED_DIRECTIONS: &str = include_str!("fixtures/conformance/pipe-rpipe-v1/actor-mixed-directions.atto");
const DUPLICATE_DIRECTION: &str = include_str!("fixtures/conformance/pipe-rpipe-v1/actor-duplicate-direction.atto");

#[test]
fn exact_canonical_actor_ports_compile_forward_and_reverse_graphs() {
    for (label, source) in [("forward", FORWARD), ("reverse", REVERSE)] {
        let guest = atto_compiler::compile(source).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(guest.starts_with(b"\0asm\x01\0\0\0"), "{label}: no guest Wasm");
    }
}

#[test]
fn exact_canonical_invalid_graphs_fail_closed_at_compiler() {
    for (label, source, diagnostic) in [
        ("missing-emits", MISSING_EMITS, "E_TYPE"),
        ("wrong-direction", WRONG_DIRECTION, "E_TYPE"),
        ("mixed-directions", MIXED_DIRECTIONS, "E_PIPE_DIRECTION"),
    ] {
        let e = atto_compiler::compile(source).expect_err(&format!("{label}: source must be rejected before guest emission"));
        assert_eq!(e.code, diagnostic, "{label}: {e}");
    }
}

#[test]
fn exact_duplicate_reverse_written_attachment_reaches_runtime_check() {
    let guest = atto_compiler::compile(DUPLICATE_DIRECTION).expect("same-directed graph requires runtime duplicate admission denial");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    // Node and Chromium must independently instantiate and observe panic_code 10.
}
