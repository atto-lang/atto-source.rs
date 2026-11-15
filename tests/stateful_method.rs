//! Actual checked state field is lowered from the AttoLang source tokens,
//! not synthesized from a preview fixture's constant. Production remains off.
use atto_compiler::target_stateful::compile_stateful_method_preview;
use atto_compiler::target_cleanup_wasm::audit_cleanup_wasm_witness;
const SOURCE:&str=include_str!("fixtures/stateful/counter.atto");

#[test]
fn real_stateful_method_has_nominal_wasm_export_and_audited_release_edges(){
    let wasm=compile_stateful_method_preview(SOURCE).unwrap();
    assert!(wasm.windows(b"atto.state.Counter.increment.1".len())
        .any(|w|w==b"atto.state.Counter.increment.1"));
    assert!(wasm.windows(b"atto.state.Counter.count".len())
        .any(|w|w==b"atto.state.Counter.count"));
    audit_cleanup_wasm_witness(&wasm).unwrap();
    assert_eq!(wasm,compile_stateful_method_preview(SOURCE).unwrap());
    // The production compiler does NOT accept this profile.
    assert_eq!(atto_compiler::compile(SOURCE).unwrap_err().code,"E_UNSUPPORTED");
    assert_eq!(atto_compiler::target_scalar::compile_scalar_receiver_instance_preview(SOURCE)
        .unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn actual_source_field_initializers_change_emitted_wasm(){
    let a=compile_stateful_method_preview(SOURCE).unwrap();
    let b=compile_stateful_method_preview(&SOURCE.replace("count = 17","count = 29")).unwrap();
    assert_ne!(a,b,"the module must initialize actual declared actor state");
    audit_cleanup_wasm_witness(&b).unwrap();
}

#[test]
fn reject_non_equivalent_stateful_callback_bodies(){
    for altered in [
        SOURCE.replace("s.count = s.count + delta","s.count = delta"),
        SOURCE.replace("s.count = s.count + delta","s.count = s.count - delta"),
        SOURCE.replace("s.count = s.count + delta","s.count = s.count + 1"),
        SOURCE.replace("s.count = s.count + delta","s.count = s.count + delta; s.count = 12"),
        SOURCE.replace("critical-write","critical-read"),
        SOURCE.replace("let int count = 17","let bool count = true"),
        SOURCE.replace("let int count = 17","let int count = 1 + 16"),
        SOURCE.replace("mtd (mut self: Counter)","mtd (Counter self)"),
        SOURCE.replace("s.count = s.count + delta","while true { s.count = delta; }"),
        SOURCE.replace("s.count = s.count + delta","s.count = s.count + delta; await sleep()"),
        format!("{SOURCE}\nfnc another(): int {{ return 3; }}"),
    ]{
        assert!(compile_stateful_method_preview(&altered).is_err(),
            "must not emit Wasm for non-admitted state callback: {altered}");
    }
}
