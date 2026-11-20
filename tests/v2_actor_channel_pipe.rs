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
