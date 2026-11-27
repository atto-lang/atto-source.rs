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
    // Keep the shared fixture bytes pinned. Its original unused loan now
    // expires under NLL; make this local witness genuinely live at drop.
    assert_eq!(source.matches("return 0;").count(), 1);
    let live = source.replace("return 0;", "return loan.value;");
    let failure = compile(&live).expect_err("live loan must prevent owner drop");
    assert_eq!(failure.code, "E_BORROW", "wrong semantic rejection: {failure}");
}

#[test]
fn expired_loan_does_not_authorize_unimplemented_destructor() {
    let source = include_str!("fixtures/alloc-parity-v1/borrow-drop-while-live-read.atto");
    let failure = compile(source).expect_err("class destruction is still unsupported");
    assert_eq!(failure.code, "E_DROP_UNSUPPORTED", "wrong semantic rejection: {failure}");
}
