//! Real Atto-to-Wasm compile gates for the experimental v2 actor/Channel bridge.
//! The Chromium test executes those guests and checks stdout, panic_code,
//! completion futures and ordered delivery; a compile-only test is NOT proof
//! of runtime behavior or multi-backend parity.

const FORWARD: &str = include_str!("fixtures/pass/208-v2-actor-channel-actor-pipe.atto");
const REVERSE: &str = include_str!("fixtures/pass/209-v2-actor-channel-actor-rpipe.atto");
const DECLARED_FORWARD: &str = include_str!("fixtures/pass/217-v2-declared-port-pipe.atto");
const DECLARED_REVERSE: &str = include_str!("fixtures/pass/218-v2-declared-port-rpipe.atto");
const DOUBLE: &str = include_str!("fixtures/panic/210-v2-pipe-double-attach.atto");
const RENDEZVOUS: &str = include_str!("fixtures/panic/211-v2-pipe-rendezvous-denial.atto");
const INPUT_OCCUPIED: &str = include_str!("fixtures/panic/212-v2-pipe-duplicate-consumer.atto");
const STOPPED_PRODUCER: &str = include_str!("fixtures/panic/213-v2-pipe-stopped-producer.atto");
const CLOSED_CHANNEL: &str = include_str!("fixtures/panic/214-v2-pipe-closed-channel.atto");
const STOPPED_CONSUMER: &str = include_str!("fixtures/panic/215-v2-pipe-stopped-consumer.atto");
const STOPPED_ACTOR_SINK: &str = include_str!("fixtures/panic/216-v2-pipe-stopped-actor-sink.atto");

#[test]
fn real_compiler_emits_independent_wasm_guests_for_both_directional_graphs() {
    for (name, source) in [("pipe", FORWARD), ("rpipe", REVERSE)] {
        let guest = atto_compiler::compile(source)
            .unwrap_or_else(|e| panic!("{name} failed to compile: {e}"));
        assert!(guest.starts_with(b"\0asm\x01\0\0\0"), "{name} is not Wasm");
        assert!(guest.len() > 8, "{name} did not produce a guest body");
    }
}

#[test]
fn canonical_declared_ports_compile_into_independent_pipe_and_rpipe_guests() {
    // These exact fixtures use `receives int emits int`, `susp receive`
    // and `pub susp fnc`; no legacy inferred `emit` grammar is required.
    for (name, source) in [
        ("canonical_pipe", DECLARED_FORWARD),
        ("canonical_rpipe", DECLARED_REVERSE),
    ] {
        let guest = atto_compiler::compile(source)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(guest.starts_with(b"\0asm\x01\0\0\0"), "{name} did not emit WebAssembly");
    }
}

#[test]
fn canonical_port_headers_are_enforced_before_guest_emission() {
    // A declared output port cannot be silently inferred from an
    // incompatible `emit(int)` body, even when the receive is int.
    for source in [DECLARED_FORWARD, DECLARED_REVERSE] {
        let wrong_output = source.replace(
            "Producer receives int emits int as", "Producer receives int emits bool as",
        );
        assert_ne!(wrong_output, source);
        let e = atto_compiler::compile(&wrong_output)
            .expect_err("wrong declared emits must reject");
        assert_eq!(e.code, "E_TYPE", "{e}");

        let wrong_input = source.replace(
            "Consumer receives int as", "Consumer receives bool as",
        );
        assert_ne!(wrong_input, source);
        let e = atto_compiler::compile(&wrong_input)
            .expect_err("declared receives and handler ActorMail must agree");
        assert_eq!(e.code, "E_TYPE", "{e}");

        let duplicate = source.replace(
            "Producer receives int emits int as",
            "Producer receives int emits int emits int as",
        );
        let e = atto_compiler::compile(&duplicate)
            .expect_err("repeated actor effect declaration must reject");
        assert_eq!(e.code, "E_DUPLICATE", "{e}");

        let missing_handler = source.replace(
            "susp receive(ActorMail<int> mail): void {\n    stdio.println(mail.value);\n  }",
            "",
        );
        assert_ne!(missing_handler, source);
        let e = atto_compiler::compile(&missing_handler)
            .expect_err("a declared receives port needs a handler");
        assert_eq!(e.code, "E_TYPE", "{e}");
    }
}

#[test]
fn observed_emit_does_not_grant_undeclared_pipeline_source_or_sink() {
    // Keeping the emit(value) body must not widen an actor's declared port
    // capabilities. The compiler must deny both spellings before Wasm output.
    for source in [DECLARED_FORWARD, DECLARED_REVERSE] {
        let missing_emits = source.replace(
            "Producer receives int emits int as", "Producer receives int as",
        );
        assert_ne!(missing_emits, source);
        let e = atto_compiler::compile(&missing_emits)
            .expect_err("body emit(value) must not infer a public DATA output");
        assert_eq!(e.code, "E_TYPE", "{e}");

        let missing_receives = source.replace(
            "Consumer receives int as", "Consumer as",
        );
        assert_ne!(missing_receives, source);
        let e = atto_compiler::compile(&missing_receives)
            .expect_err("receive handler must not grant an undeclared DATA input");
        assert_eq!(e.code, "E_TYPE", "{e}");
    }
}

#[test]
fn executable_negative_fixtures_also_compile_to_guests_that_must_trap() {
    // Runtime graph-admission failure cannot be inferred from compiler
    // rejection. Chromium must execute these guests and observe panic 10.
    for (name, source) in [
        ("double", DOUBLE),
        ("rendezvous", RENDEZVOUS),
        ("input_occupied", INPUT_OCCUPIED),
        ("stopped_producer", STOPPED_PRODUCER),
        ("closed_channel", CLOSED_CHANNEL),
        ("stopped_consumer", STOPPED_CONSUMER),
        ("stopped_actor_sink", STOPPED_ACTOR_SINK),
    ] {
        let guest = atto_compiler::compile(source)
            .unwrap_or_else(|e| panic!("{name} must reach runtime: {e}"));
        assert!(guest.starts_with(b"\0asm\x01\0\0\0"), "{name} is not Wasm");
    }
}

#[test]
fn actor_channel_type_mismatch_rejects_before_emission_in_both_directions() {
    let mismatched = FORWARD.replace("Channel<int>.bounded(1)", "Channel<bool>.bounded(1)");
    let forward = atto_compiler::compile(&mismatched)
        .expect_err("actor int output cannot enter a bool channel");
    assert_eq!(forward.code, "E_TYPE", "{forward}");

    let mismatched = REVERSE.replace("Channel<int>.bounded(1)", "Channel<bool>.bounded(1)");
    let reverse = atto_compiler::compile(&mismatched)
        .expect_err("reverse-written bool channel cannot take actor int");
    assert_eq!(reverse.code, "E_TYPE", "{reverse}");
}

#[test]
fn invalid_actor_sink_receives_type_fails_before_emitting_guest() {
    for source in [FORWARD, REVERSE] {
        let bad = source.replace(
            "receive(ActorMail<int> mail): void {\n    stdio.println(mail.value);",
            "receive(ActorMail<bool> mail): void {\n    stdio.println(mail.value);",
        );
        assert_ne!(bad, source, "fixture changed its consumer syntax");
        let e = atto_compiler::compile(&bad)
            .expect_err("cannot route channel int to bool consumer");
        assert_eq!(e.code, "E_TYPE", "{e}");
    }
}

#[test]
fn mixed_direction_without_parentheses_is_a_compile_error() {
    let bad = FORWARD.replace(
        "producer pipe channel pipe consumer",
        "producer pipe channel rpipe consumer",
    );
    assert_ne!(bad, FORWARD);
    let e = atto_compiler::compile(&bad)
        .expect_err("unparenthesized mixed direction must fail closed");
    assert_eq!(e.code, "E_PIPE_DIRECTION", "{e}");
}


#[test]
fn nested_four_stage_graphs_fail_closed_before_prefix_emission() {
    // Only an exact actor/Channel/actor triple has the experimental
    // atomic guest setup helper. A fourth stage must not recursively
    // activate the first three and then fail a later admission.
    for (label, fixture, original, longer) in [
        ("pipe", FORWARD,
         "producer pipe channel pipe consumer",
         "producer pipe channel pipe consumer pipe consumer"),
        ("rpipe", REVERSE,
         "consumer rpipe channel rpipe producer",
         "consumer rpipe channel rpipe producer rpipe producer"),
    ] {
        let source = fixture.replace(original, longer);
        assert_ne!(source, fixture, "{label}: source must change");
        let e = atto_compiler::compile(&source)
            .expect_err("four-stage graph must fail before Wasm emission");
        assert_eq!(e.code, "E_UNSUPPORTED", "{label}: {e}");
    }
}


// Provisional compile gates. The Node/Chromium suites MUST execute the
// resulting Wasm guest before counting these as checked runtime passes.
const CHECKED_FORWARD: &str = include_str!("fixtures/pass/219-v2-checked-actor-channel-actor-pipe.atto");
const CHECKED_REVERSE: &str = include_str!("fixtures/pass/220-v2-checked-actor-channel-actor-rpipe.atto");
const CHECKED_ROLLBACK: &str = include_str!("fixtures/pass/221-v2-checked-setup-rollback.atto");
const CHECKED_OUTPUT_OCCUPIED: &str = include_str!("fixtures/pass/222-v2-checked-setup-output-occupied.atto");
const CHECKED_OUTPUT_REUSE: &str = include_str!("fixtures/pass/222-v2-checked-output-occupied-rollback.atto");
const CHECKED_INPUT_REUSE: &str = include_str!("fixtures/pass/223-v2-checked-input-occupied-rollback.atto");

#[test]
fn annotated_pipe_handle_result_compiles_both_directions_and_failed_setup_rollback() {
    for (name, source) in [
        ("checked_forward", CHECKED_FORWARD),
        ("checked_reverse", CHECKED_REVERSE),
        ("checked_rollback", CHECKED_ROLLBACK),
        ("checked_output_occupied", CHECKED_OUTPUT_OCCUPIED),
        ("checked_output_occupied_rollback", CHECKED_OUTPUT_REUSE),
        ("checked_input_occupied_rollback", CHECKED_INPUT_REUSE),
    ] {
        let guest = atto_compiler::compile(source)
            .unwrap_or_else(|e| panic!("{name}: checked graph failed compiler: {e}"));
        assert!(guest.starts_with(b"\0asm\x01\0\0\0"),
            "{name}: checked graph did not emit Wasm");
    }
}

#[test]
fn opaque_pipe_handle_is_not_constructible_from_an_integer() {
    let bad = CHECKED_FORWARD.replace(
        "producer pipe channel pipe consumer",
        "Ok(42)",
    );
    assert_ne!(bad, CHECKED_FORWARD);
    let e = atto_compiler::compile(&bad)
        .expect_err("a guest integer must not forge an opaque PipeHandle");
    assert_eq!(e.code, "E_TYPE", "{e}");
}

#[test]
fn typed_pipe_handle_is_not_implicitly_unwrapped_to_channel() {
    let bad = CHECKED_FORWARD.replace(
        "Result<PipeHandle, PipeSetupError> connection",
        "Channel<int> connection",
    );
    assert_ne!(bad, CHECKED_FORWARD);
    let e = atto_compiler::compile(&bad)
        .expect_err("an explicit Channel<T> binding must not become PipeHandle");
    // The legacy raw Channel<T> guest surface remains a separate experimental
    // contract. Reject the resulting invalid match pattern, rather than
    // pretending typed Result is silently available.
    assert!(e.code == "E_TYPE" || e.code == "E_MATCH", "{e}");
}


#[test]
fn unverified_pipe_handle_cancel_is_rejected_before_wasm_emission() {
    let bad = CHECKED_FORWARD.replace(
        "on Ok(_) -> 1,",
        "on Ok(handle) -> handle.cancel(),",
    );
    assert_ne!(bad, CHECKED_FORWARD);
    let e = atto_compiler::compile(&bad)
        .expect_err("guest must not cancel a PipeHandle without scheduler cleanup support");
    assert_eq!(e.code, "E_UNSUPPORTED", "{e}");
}
