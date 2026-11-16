//! Regression: a 250-term left-associative expression must compile
//! on the DEFAULT Rust test-thread stack. RUST_MIN_STACK is not raised.
use atto_compiler::compile;

fn chain(n: usize, op: &str) -> String {
    format!("pub fnc main(): int {{ return {}; }}", vec!["1"; n].join(op))
}

#[test]
fn two_hundred_fifty_additions_compile_without_stack_overflow() {
    let input = chain(250, " + ");
    let wasm = compile(&input).expect("250 left-associative additions must compile");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(compile(&input).expect("deterministic repeat"), wasm);
}

#[test]
fn very_long_operator_chains_fail_closed_on_default_stack() {
    for op in [" + ", " * ", " - ", " == ", " < "] {
        let error = compile(&chain(3_000, op)).expect_err("3,000 operators require a nesting diagnostic");
        assert_eq!(error.code, "E_NESTING", "{op}: {error}");
    }
    assert!(compile("pub fnc main(): int { return 10 - 3 - 2; }").is_ok());
}
