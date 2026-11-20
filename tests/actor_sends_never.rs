//! Executable source-compiler regression for the empty actor DATA effect.
//! This validates real compiler output, not mailbox guest execution.
use atto_compiler::compile;
use atto_compiler::{ast::{Type, ActorSendEffect}, check, lexer, parser::Parser};

const BASE: &str = r#"define actor Worker receives int as
    receive(ActorMail<int> mail): void { return; }
end
pub fnc main(): int { return 0; }
"#;

#[test]
fn sends_never_compiles_into_a_real_wasm_artifact() {
    let source = BASE.replace("receives int as", "receives int sends never as");
    let bytes = compile(&source).expect("an actor with no outbound sends permits sends never");
    assert!(bytes.len() > 8);
    assert!(bytes.starts_with(b"\0asm"));
    // The non-effect annotation must also be valid without an explicit
    // receives clause in the bounded legacy compatibility profile.
    let legacy = BASE.replace("receives int as", "sends never as");
    assert!(compile(&legacy).expect("legacy actor with no outbound sends").starts_with(b"\0asm"));
}

#[test]
fn sends_void_remains_a_compile_error_not_an_alias_for_never() {
    let source = BASE.replace("receives int as", "receives int sends void as");
    assert_eq!(compile(&source).unwrap_err().code, "E_ACTOR_SEND_CONTRACT");
}

#[test]
fn duplicate_and_out_of_order_never_clauses_fail_closed() {
    for header in [
        "receives int sends never sends int",
        "sends never receives int",
        "receives int sends int sends never",
    ] {
        let source = BASE.replace("receives int", header);
        assert!(compile(&source).is_err(), "{header} must not be accepted");
    }
}

#[test]
fn explicit_sends_never_does_not_change_the_destination_inbox() {
    let source = BASE
        .replace("receives int as", "receives int sends never as")
        .replace("ActorMail<int>", "ActorMail<bool>");
    assert_eq!(compile(&source).unwrap_err().code, "E_ACTOR_RECEIVES");
}

#[test]
fn canonical_bool_never_fixture_compiles_to_wasm() {
    let source = include_str!("fixtures/pass/actor-bool-never-parity-v1.atto");
    let wasm = compile(source).expect("bool inbox and empty outbound effect must compile");
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn never_is_distinct_from_void_and_forged_void_payload_fails_in_checker() {
    let source = BASE.replace("receives int as", "receives int sends never as");
    let mut parser = Parser::new(lexer::lex(&source).unwrap());
    let parsed = parser.parse_program().unwrap();
    assert_eq!(parsed.actors[0].sends, Some(ActorSendEffect::Never));
    let mut corrupted = parsed.clone();
    corrupted.actors[0].sends = Some(ActorSendEffect::Payload(Type::Void));
    assert_eq!(check::check(&corrupted).unwrap_err().code, "E_ACTOR_SEND_CONTRACT");
}

#[test]
fn consuming_take_cannot_be_sent_through_void_admission_abi() {
    for expression in ["take! value", "(take! value) + 1", "take! value + 1"] {
        let source = BASE.replace(
            "return 0; }",
            &format!("val value = 42; val worker = spawn Worker(); worker.send({expression}); return 0; }}")
        );
        let err = compile(&source).expect_err("no artifact without transactional ownership acknowledgement");
        assert_eq!(err.code, "E_ACTOR_TRANSFER_UNSUPPORTED", "{expression}");
    }
}

#[test]
fn scalar_send_without_take_retains_existing_guest_compilation() {
    let source = BASE.replace("return 0; }",
        "val value = 42; val worker = spawn Worker(); worker.send(value); return 0; }");
    assert!(compile(&source).expect("copyable scalar send remains allowed").starts_with(b"\0asm"));
}

#[test]
fn actor_isolation_negative_fixtures_never_emit_wasm() {
    // A successful compile would return an artifact, so every attempted
    // negative case must instead yield its stable, declared diagnostic.
    const NEGATIVES: &[(&str, &str)] = &[
        (include_str!("fixtures/fail/actor-isolation-pointer-mail-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-isolation-arc-mail-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-isolation-mutex-mail-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-isolation-nominal-mail-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-isolation-sendable-spoof-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-isolation-shareable-spoof-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-isolation-serializable-spoof-v1.atto"), "E_UNSUPPORTED_TYPE"),
        (include_str!("fixtures/fail/actor-untrusted-no-sandbox-v1.atto"), "E_UNTRUSTED_SANDBOX_UNAVAILABLE"),
        (include_str!("fixtures/fail/actor-isolated-no-private-runtime-v1.atto"), "E_ACTOR_DOMAIN_RUNTIME_UNAVAILABLE"),
    ];
    for (index, (source, expected_code)) in NEGATIVES.iter().enumerate() {
        let err = compile(source).expect_err("invalid isolation contract must not emit guest bytes");
        assert_eq!(err.code, *expected_code, "negative isolation fixture {index}");
    }
}

#[test]
fn forged_receive_ast_cannot_widen_compiler_scalar_witness() {
    let mut parser = Parser::new(lexer::lex(BASE).unwrap());
    let program = parser.parse_program().unwrap();
    for forbidden in [Type::Float, Type::Str, Type::ActorRef(0), Type::OptionInt] {
        let mut tampered = program.clone();
        tampered.actors[0].receives = Some(forbidden);
        tampered.actors[0].receive.as_mut().unwrap().payload = forbidden;
        let err = check::check(&tampered)
            .expect_err("typed checker must reject forged actor mailbox shapes");
        assert_eq!(err.code, "E_UNSUPPORTED_TYPE", "forged mailbox {forbidden:?}");
    }
}

#[test]
fn runtime_scalar_witness_does_not_weaken_take_send_gate() {
    let source = BASE.replace("return 0; }",
        "val worker = spawn Worker(); val value = 42; worker.send(take! value); return 0; }");
    let err = compile(&source).expect_err("copyable scalar is not a permit for take! escrow");
    assert_eq!(err.code, "E_ACTOR_TRANSFER_UNSUPPORTED");
}
