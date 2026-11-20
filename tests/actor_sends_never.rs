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
    assert!(compile(&source).expect("copyable scalar send remains allowed").starts_with(b"\\0asm"));
}
