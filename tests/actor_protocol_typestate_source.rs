//! Actual .atto source preflight tests: NOT guest actor execution.
//! Every accepted protocol still fails closed at the Wasm emission boundary.
use atto_compiler::{actor_protocol_typestate::{inspect_source, Response}, compile};

// Exactly the Git blob used by atto-platform-conformance, not a retyped copy.
const GOOD: &str = include_str!("fixtures/actor-protocol-typestate-v1/connection-static-valid.atto");

fn rejects(src: &str, expected: &'static str) {
    let err = compile(src).expect_err("protocol source cannot emit unverified guest code");
    assert_eq!(err.code, expected, "{err}");
    assert!(err.pos.line > 0 && err.pos.col > 0, "{err}");
}

#[test]
fn connection_protocol_is_checked_from_actual_atto_tokens() {
    let proof = inspect_source(GOOD).expect("static protocol").expect("protocol declaration");
    assert_eq!(proof.completed_handles, 1);
    assert_eq!(proof.actors.get("Connection").map(String::as_str), Some("ConnectionProtocol"));
    let p = &proof.protocols["ConnectionProtocol"];
    assert_eq!(p.initial, "Disconnected");
    assert!(p.terminal.contains("Closed"));
    assert_eq!(p.transitions.len(), 5);
    assert_eq!(p.check_send(&["Disconnected".into()], "Connect").unwrap().next_states, vec!["Connected".to_string()]);
    assert_eq!(p.check_send(&["Connected".into()], "Query").unwrap().response,
        Response::Unary("QueryReply".into()));
    assert_eq!(p.check_send(&["Connected".into(), "Disconnected".into()], "Close")
        .unwrap().next_states, vec!["Closed".to_string()]);
    assert_eq!(p.check_send(&["Disconnected".into()], "Query").unwrap_err().code,
        "E_PROTOCOL_ILLEGAL_MESSAGE_STATE");
    assert_eq!(p.check_send(&["Connected".into(), "Disconnected".into()], "Query")
        .unwrap_err().code, "E_PROTOCOL_ILLEGAL_MESSAGE_STATE");
    assert_eq!(p.check_send(&[], "Query").unwrap_err().code, "E_PROTOCOL_UNPROVEN_STATE");
}

#[test]
fn validated_source_cannot_emit_a_fake_actor_guest() {
    rejects(GOOD, "E_PROTOCOL_RUNTIME_UNSUPPORTED");
}

#[test]
fn disconnected_query_is_a_compile_error_not_runtime_fallback() {
    let bad = GOOD.replace("disconnected.call(Connect())", "disconnected.call(Query())");
    rejects(&bad, "E_PROTOCOL_ILLEGAL_MESSAGE_STATE");
}

#[test]
fn duplicate_or_consumed_handle_is_rejected() {
    let bad = GOOD.replace("val querying = connected.call(Query());",
        "val querying = disconnected.call(Query());");
    rejects(&bad, "E_PROTOCOL_AFFINE_HANDLE");
    let bad = GOOD.replace("val querying = connected.call(Query());",
        "val duplicate = connected;\n  val querying = connected.call(Query());");
    rejects(&bad, "E_PROTOCOL_AFFINE_HANDLE");
}

#[test]
fn missing_reply_and_early_completion_are_not_proven_success() {
    let bad = GOOD.replace("val (ack, connected) = await connecting?;",
        "val (ack, connected) = await not_connecting?;");
    rejects(&bad, "E_PROTOCOL_REPLY_REQUIRED");
    let bad = GOOD.replace("val connecting = disconnected.call(Connect());",
        "disconnected.complete_protocol();\n  val connecting = disconnected.call(Connect());");
    rejects(&bad, "E_PROTOCOL_TERMINAL_INCOMPLETE");
    let bad = GOOD.replace("val (ack, connected) = await connecting?;",
        "val (ack, connected) = await connecting;");
    rejects(&bad, "E_PROTOCOL_REPLY_REQUIRED");
}

#[test]
fn unknown_modes_or_nominal_response_disagreement_fail_closed() {
    let bad = GOOD.replace("replies QueryReply", "streams QueryReply");
    rejects(&bad, "E_PROTOCOL_REPLY_MISMATCH");
    let bad = GOOD.replace("on Query(Sql) -> Connected replies QueryReply",
        "on Query(Sql) -> Connected");
    rejects(&bad, "E_PROTOCOL_REPLY_REQUIRED");
    let bad = GOOD.replace("on Query(Sql) -> Connected replies QueryReply",
        "on Query(Sql) -> Connected replies QueryReply\n    on Query(Sql) -> Closed replies QueryReply");
    rejects(&bad, "E_PROTOCOL_DUPLICATE");
}

#[test]
fn wrong_nominal_implementer_is_rejected() {
    let bad = GOOD.replace("define actor Connection implements", "define class Connection implements");
    rejects(&bad, "E_PROTOCOL_IMPLEMENTATION_MISMATCH");
}

#[test]
fn declarative_state_graph_is_validated_and_bounded() {
    rejects(&GOOD.replace("terminal state Closed", "terminal state Never"), "E_PROTOCOL_STATE");
    rejects(&GOOD.replace("initial Disconnected", "initial Unknown"), "E_PROTOCOL_STATE");
    rejects(&GOOD.replace("state Connected as", "state Never as"), "E_PROTOCOL_STATE");
    rejects(&GOOD.replace("on Close -> Closed replies ClosedAck", "on Close -> Disconnected replies ClosedAck"),
        "E_PROTOCOL_STATE");
    rejects(&GOOD.replace("terminal state Closed", "terminal state Closed\n  terminal state Closed"),
        "E_PROTOCOL_DUPLICATE");
}

#[test]
fn lexical_comments_and_strings_do_not_declare_protocols() {
    let innocent = r#"
      // define protocol Hidden as
      /* define protocol Other as */
      pub fnc main(): int { val marker = "define protocol Ghost as"; return 0; }
    "#;
    assert!(inspect_source(innocent).unwrap().is_none());
}
#[test]
fn forged_protocol_handle_without_a_definition_fails_closed() {
    rejects("pub fnc main(): int { val h: ProtocolHandle<Connection,Connected> = none; return 0; }",
        "E_PROTOCOL_UNPROVEN_STATE");
}
#[test]
fn unrelated_scalar_compilation_remains_executable() {
    let bytes = compile("pub fnc main(): int { return 42; }").expect("independent scalar guest");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn live_authority_cannot_escape_by_argument_return_or_unchecked_call() {
    rejects(&GOOD.replace("val connecting = disconnected.call(Connect());",
        "ordinary(disconnected);\n  val connecting = disconnected.call(Connect());"),
        "E_PROTOCOL_CFG_UNSUPPORTED");
    rejects(&GOOD.replace("val connecting = disconnected.call(Connect());",
        "disconnected.send(Connect());\n  val connecting = disconnected.call(Connect());"),
        "E_PROTOCOL_UNPROVEN_STATE");
    rejects(&GOOD.replace("val connecting = disconnected.call(Connect());",
        "val owned = copy! disconnected;\n  val connecting = disconnected.call(Connect());"),
        "E_PROTOCOL_AFFINE_HANDLE");
}

#[test]
fn a_completed_ticket_name_cannot_be_reissued() {
    let bad = GOOD.replace("val querying = connected.call(Query());",
        "val connecting = connected.call(Query());");
    rejects(&bad, "E_PROTOCOL_AFFINE_HANDLE");
}

#[test]
fn mutable_loan_of_a_protocol_handle_is_rejected() {
    rejects(&GOOD.replace("val disconnected = await spawn Connection();",
        "let disconnected = await spawn Connection();"),
        "E_PROTOCOL_AFFINE_HANDLE");
}

#[test]
fn public_protocol_visibility_spelling_is_accepted_only_as_type_contract() {
    for spelling in ["pub define protocol", "define pub protocol"] {
        let source = GOOD.replacen("define protocol", spelling, 1);
        assert!(inspect_source(&source).unwrap().is_some());
        rejects(&source, "E_PROTOCOL_RUNTIME_UNSUPPORTED");
    }
}

#[test]
fn required_nominal_reply_types_are_checked_before_successor_is_minted() {
    let typed = GOOD.replace("val (ack, connected) = await connecting?;",
        "val (ack: ConnectedAck, connected) = await connecting?;");
    assert_eq!(inspect_source(&typed).unwrap().unwrap().completed_handles, 1);
    rejects(&typed, "E_PROTOCOL_RUNTIME_UNSUPPORTED");
    let wrong = GOOD.replace("val (ack, connected) = await connecting?;",
        "val (ack: WrongAck, connected) = await connecting?;");
    rejects(&wrong, "E_PROTOCOL_REPLY_MISMATCH");
    let mutable = GOOD.replace("val (ack, connected) = await connecting?;",
        "let (ack: ConnectedAck, connected) = await connecting?;");
    rejects(&mutable, "E_PROTOCOL_AFFINE_HANDLE");
}

#[test]
fn identical_source_conformance_negatives_are_compiler_diagnostics() {
    for (source, code) in [
        (include_str!("fixtures/actor-protocol-typestate-v1/disconnected-query.atto"), "E_PROTOCOL_ILLEGAL_MESSAGE_STATE"),
        (include_str!("fixtures/actor-protocol-typestate-v1/affine-reuse.atto"), "E_PROTOCOL_AFFINE_HANDLE"),
        (include_str!("fixtures/actor-protocol-typestate-v1/missing-reply.atto"), "E_PROTOCOL_REPLY_REQUIRED"),
        (include_str!("fixtures/actor-protocol-typestate-v1/early-terminal.atto"), "E_PROTOCOL_TERMINAL_INCOMPLETE"),
        (include_str!("fixtures/actor-protocol-typestate-v1/stream-unary-confusion.atto"), "E_PROTOCOL_REPLY_MISMATCH"),
        (include_str!("fixtures/actor-protocol-typestate-v1/wrong-actor-kind.atto"), "E_PROTOCOL_IMPLEMENTATION_MISMATCH"),
        (include_str!("fixtures/actor-protocol-typestate-v1/wrong-nominal-reply.atto"), "E_PROTOCOL_REPLY_MISMATCH"),
    ] {
        rejects(source, code);
    }
    let typed = include_str!("fixtures/actor-protocol-typestate-v1/typed-reply-static-valid.atto");
    assert_eq!(inspect_source(typed).unwrap().unwrap().completed_handles, 1);
    rejects(typed, "E_PROTOCOL_RUNTIME_UNSUPPORTED");
}

#[test]
fn forward_actor_declaration_may_match_exact_protocol_definition() {
    let src = GOOD.replace("define actor Connection implements ConnectionProtocol as end",
        "declare actor Connection implements ConnectionProtocol;\n" +
        "define actor Connection implements ConnectionProtocol as end");
    assert_eq!(inspect_source(&src).unwrap().unwrap().completed_handles, 1);
    rejects(&src, "E_PROTOCOL_RUNTIME_UNSUPPORTED");
    let bad = GOOD.replace("define actor Connection implements ConnectionProtocol as end",
        "define actor Connection implements ConnectionProtocol as end\n" +
        "define actor Connection implements ConnectionProtocol as end");
    rejects(&bad, "E_PROTOCOL_DUPLICATE");
}

#[test]
fn nonlinear_control_flow_never_becomes_an_accidental_linear_proof() {
    for guard in [
        "if true then",
        "loop",
        "try",
        "while true do",
        "match 1 over",
        "select",
    ] {
        let bad = GOOD.replace("val disconnected = await spawn Connection();",
            &format!("{guard}\n  val disconnected = await spawn Connection();"));
        rejects(&bad, "E_PROTOCOL_CFG_UNSUPPORTED");
    }
}

#[test]
fn early_return_and_unverified_effects_do_not_preserve_authority() {
    let early = GOOD.replace("val connecting = disconnected.call(Connect());",
        "return 1;\n  val connecting = disconnected.call(Connect());");
    rejects(&early, "E_PROTOCOL_CFG_UNSUPPORTED");
    let before_spawn = GOOD.replace("val disconnected = await spawn Connection();",
        "return 0;\n  val disconnected = await spawn Connection();");
    rejects(&before_spawn, "E_PROTOCOL_CFG_UNSUPPORTED");
    for operation in [
        "await external_task();",
        "spawn Other();",
        "unknown_side_effect();",
        "{ unrelated(); }",
    ] {
        let bad = GOOD.replace("val connecting = disconnected.call(Connect());",
            &format!("{operation}\n  val connecting = disconnected.call(Connect());"));
        rejects(&bad, "E_PROTOCOL_CFG_UNSUPPORTED");
    }
}

#[test]
fn message_payload_and_spawn_constructor_cannot_hide_affine_handles_or_effects() {
    let aliased = GOOD.replace("disconnected.call(Connect())",
        "disconnected.call(Connect(disconnected))");
    rejects(&aliased, "E_PROTOCOL_AFFINE_HANDLE");
    let side_effect = GOOD.replace("disconnected.call(Connect())",
        "disconnected.call(Connect(revoke_authority()))");
    rejects(&side_effect, "E_PROTOCOL_CFG_UNSUPPORTED");
    let field_getter = GOOD.replace("disconnected.call(Connect())",
        "disconnected.call(Connect(config.value))");
    rejects(&field_getter, "E_PROTOCOL_CFG_UNSUPPORTED");
    let bad_spawn = GOOD.replace("await spawn Connection()",
        "await spawn Connection(revoke_authority())");
    rejects(&bad_spawn, "E_PROTOCOL_CFG_UNSUPPORTED");
    let missing_parentheses = GOOD.replace("await spawn Connection()",
        "await spawn Connection");
    rejects(&missing_parentheses, "E_PROTOCOL_SYNTAX");
}

#[test]
fn unimplemented_actor_and_reserved_state_names_fail_closed() {
    let declaration_only = GOOD.replace(
        "define actor Connection implements ConnectionProtocol as end",
        "declare actor Connection implements ConnectionProtocol;");
    rejects(&declaration_only, "E_PROTOCOL_UNPROVEN_STATE");
    let reserved = GOOD.replace("initial Disconnected", "initial if");
    rejects(&reserved, "E_PROTOCOL_SYNTAX");
    let reserved_protocol = GOOD.replace("define protocol ConnectionProtocol",
        "define protocol state");
    rejects(&reserved_protocol, "E_PROTOCOL_SYNTAX");
}
