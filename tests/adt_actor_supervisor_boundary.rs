//! Actor/ADT boundary regression matrix for the *current* restricted Rust/Wasm
//! compiler. These tests intentionally distinguish executable scalar ADTs from
//! ADT mailbox support, which is NOT implemented yet. A future implementation
//! must replace the rejection assertions with compiled-guest delivery/CONTROL
//! lifecycle tests rather than deleting them.
use atto_compiler::compile;

fn reject(source: &str, expected: Option<&str>) {
    let error = compile(source).expect_err("unsupported or malformed ADT must produce no guest artifact");
    assert!(error.code.starts_with("E_"), "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
    if let Some(code) = expected {
        assert_eq!(error.code, code, "{error}");
    }
}

fn actor_source(payload_type: &str) -> String {
    format!(
        "define actor Sink as\\n  receive(ActorMail<{payload_type}> mail): void {{ return; }}\\nend\\npub fnc main(): int {{ return 0; }}"
    ).replace("\\n", "\n")
}

#[test]
fn scalar_actor_mail_remains_executable() {
    for payload_type in ["int", "bool"] {
        let source = actor_source(payload_type);
        let wasm = compile(&source).unwrap_or_else(|e| panic!("{payload_type}: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{payload_type}");
        assert_eq!(compile(&source).unwrap(), wasm, "guest output must be deterministic");
    }
}

#[test]
fn every_currently_unsupported_adt_mailbox_shape_fails_closed() {
    // The browser compiler currently has no guest/host wire ABI for these
    // payloads, even though some can be used in ordinary scalar functions.
    for ty in [
        "Option<int>",
        "Result<int,string>",
        "Option<Result<int,string>>",
        "Option<Option<int>>",
        "Result<bool,string>",
        "Result<int,int>",
        "Option<bool>",
    ] {
        let source = actor_source(ty);
        reject(&source, Some("E_UNSUPPORTED_TYPE"));
    }
    assert!(compile(&actor_source("int")).is_ok(), "rejected mail must not poison compiler state");
}

#[test]
fn adt_constructors_cannot_be_smuggled_into_a_scalar_actor_send() {
    for expression in [
        "Some(42)",
        "None",
        "Ok(42)",
        "Err(\"failed\")",
        "Some(Err(\"failed\"))",
    ] {
        let source = format!(
            "define actor Sink as receive(ActorMail<int> mail): void {{ return; }} end \
             pub fnc main(): int {{ val a = spawn Sink(); a.send({expression}); return 0; }}"
        );
        reject(&source, None);
    }
}

#[test]
fn nested_adt_value_profiles_still_compile_outside_actor_mail() {
    for src in [
        include_str!("fixtures/pass/91-nested-option-result-ok.atto"),
        include_str!("fixtures/pass/92-nested-option-result-err.atto"),
        include_str!("fixtures/pass/93-nested-option-result-none.atto"),
        include_str!("fixtures/pass/94-nested-option-result-in-arguments.atto"),
    ] {
        let wasm = compile(src).expect("non-mailbox nested ADT profile must remain executable");
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    }
}

#[test]
fn malformed_nested_adts_never_emit_a_module_and_valid_compilation_recovers() {
    for src in [
        include_str!("fixtures/fail/102-adt-identity-equality.atto"),
        include_str!("fixtures/fail/104-adt-wrong-variant-family.atto"),
        include_str!("fixtures/fail/105-adt-none-pattern-arity.atto"),
        include_str!("fixtures/fail/106-nested-option-result-wrong-payload.atto"),
        include_str!("fixtures/fail/107-nested-option-result-nonexhaustive.atto"),
        include_str!("fixtures/fail/108-nested-option-result-inner-nonexhaustive.atto"),
        include_str!("fixtures/fail/110-nested-adt-reference-equality.atto"),
    ] {
        reject(src, None);
    }
    assert!(compile("pub fnc main(): int { return 42; }").is_ok());
}
