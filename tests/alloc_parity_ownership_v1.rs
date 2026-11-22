//! Source-compiled rejection witnesses for the shared allocation parity contract.
//! Tests must fail on incorrect diagnostic codes, not just an arbitrary compiler
//! panic or unrelated syntax error. No guest execution or AOT is asserted.
use atto_compiler::compile;

#[test]
fn ownership_double_drop_rejected() {
    let source = include_str!("fixtures/alloc-parity-v1/ownership-double-drop.atto");
    let failure = compile(source).expect_err("double drop must not produce a Wasm artifact");
    assert_eq!(failure.code, "E_MOVED", "wrong semantic rejection: {failure}");
}

#[test]
fn borrow_drop_while_live_read_rejected() {
    let source = include_str!("fixtures/alloc-parity-v1/borrow-drop-while-live-read.atto");
    let failure = compile(source).expect_err("live loan must prevent owner drop");
    assert_eq!(failure.code, "E_BORROW", "wrong semantic rejection: {failure}");
}
