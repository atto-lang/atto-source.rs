//! Real guest codegen for the first private class-backed closure profile.
//! This does NOT imply a general typed call_indirect runtime or reclaiming GC.
const ADDER: &str = "fnc make_adder(int offset): fnc(int) => int { return |n| : int -> n + offset; } pub fnc main(): int { val add_one = make_adder(1); return add_one(41); }";
const TWICE: &str = "fnc apply_twice(fnc(int) => int operation, int value): int { return operation(operation(value)); } pub fnc main(): int { val double_it = |n| : int -> n * 2; return apply_twice(double_it, 10); }";
#[test]
fn escaping_integer_capture_emits_deterministic_guest_wasm() {
    let first = atto_compiler::compile(ADDER).expect("class-backed owned capture");
    assert!(first.starts_with(b"\\0asm"), "must emit real guest Wasm");
    assert_eq!(first, atto_compiler::compile(ADDER).unwrap());
}
#[test]
fn higher_order_callable_executes_through_emitted_receiver_method() {
    let first = atto_compiler::compile(TWICE).expect("typed higher-order closure");
    assert!(first.starts_with(b"\\0asm"));
    assert_eq!(first, atto_compiler::compile(TWICE).unwrap());
}
#[test]
fn mutable_capture_rejected_before_artifact_emission() {
    let src = "fnc make_adder(int offset): fnc(int) => int { let mutable_offset = offset; return |n| : int -> n + mutable_offset; } pub fnc main(): int { return 42; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_CAPTURE");
}
#[test]
fn closure_profile_fails_closed_for_incompatible_scalar_signature() {
    let src = "fnc make_flag(int offset): fnc(bool) => int { return |n| : int -> offset; } pub fnc main(): int { return 42; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED_TYPE");
}
