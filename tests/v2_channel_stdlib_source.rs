//! Source-package guard, not a substitute for guest execution tests.
const LIB: &str = include_str!("../stdlib/channel.atto");
const PRELUDE: &str = include_str!("../src/v2/prelude.atto");
const SLOT: &str = "// @atto-stdlib-channel-source";
const CHECKER: &str = include_str!("../src/v2/check.rs");
const INVALID_SENTINEL: &str =
    include_str!("fixtures/panic/235-v2-negative-bounded-channel.atto");
const INVALID_OTHER: &str =
    include_str!("fixtures/panic/236-v2-negative-other-capacity.atto");
const INVALID_LEGACY: &str =
    include_str!("fixtures/panic/237-v2-negative-legacy-channel-constructor.atto");
const VALID_UNBOUNDED: &str =
    include_str!("fixtures/pass/236-v2-unbounded-constructor-atto.atto");
const RENDEZVOUS_DIRECT: &str =
    include_str!("fixtures/pass/238-v2-rendezvous-direct-readch.atto");
const RENDEZVOUS_ITER: &str =
    include_str!("fixtures/pass/239-v2-rendezvous-for-await.atto");
const RENDEZVOUS_CLOSE: &str =
    include_str!("fixtures/pass/240-v2-rendezvous-close-wakes-reader.atto");
const LEGACY_PENDING_PRODUCER: &str =
    include_str!("fixtures/panic/241-v2-legacy-pipe-producer-pending-read-denial.atto");
const LEGACY_PENDING_CONSUMER: &str =
    include_str!("fixtures/panic/242-v2-legacy-pipe-consumer-pending-read-denial.atto");
const LEGACY_PRELOADED_PRODUCER: &str =
    include_str!("fixtures/panic/243-v2-legacy-pipe-producer-preloaded-denial.atto");
const LEGACY_PRELOADED_CONSUMER: &str =
    include_str!("fixtures/panic/244-v2-legacy-pipe-consumer-preloaded-denial.atto");


#[test]
fn channel_module_is_extracted_once_and_not_duplicated_in_prelude() {
    assert_eq!(PRELUDE.matches(SLOT).count(), 1);
    assert!(!PRELUDE.contains("fnc __chan_new("));
    assert!(!PRELUDE.contains("fnc __chan_send("));
    let declarations = [
        "__chan_new", "__chan_new_bounded", "__chan_offer", "__chan_send", "__chan_send_pipe",
        "__chan_take", "__chan_recv", "__chan_recv_user_opt",
        "__chan_recv_opt", "__chan_try", "__chan_nb",
        "__chan_close_internal", "__chan_close",
    ];
    for name in declarations {
        assert_eq!(LIB.matches(&format!("fnc {name}(")).count(), 1,
            "the channel source must define {name} exactly once");
    }
    assert!(!LIB.contains("import java:"));
    assert!(!LIB.contains("import jvm"));
    assert!(!LIB.contains("require("));
    assert!(!LIB.contains("host_call"));
}

#[test]
fn channel_module_is_compiler_injected_at_the_original_runtime_boundary() {
    let composed = PRELUDE.replacen(SLOT, LIB, 1);
    assert!(composed.contains("fnc __chan_close(int c): void"));
    assert!(composed.contains("fnc __actor_stop(int a): void"));
    assert!(!composed.contains(SLOT));
    let channel = composed.find("fnc __chan_new(int cap)").unwrap();
    let actors = composed.find("// ------------------------------------------------------------------ actors").unwrap();
    let futures = composed.find("// ----------------------------------------------------------------- futures").unwrap();
    assert!(futures < channel && channel < actors);
}

#[test]
fn checked_bounded_constructor_cannot_alias_unbounded_sentinel() {
    assert!(CHECKER.contains("self.rt(\"__chan_new_bounded\", vec![n])"),
        "bounded() must call the checked guest .atto constructor");
    assert!(CHECKER.contains("self.rt(\"__chan_new\", vec![Ir::Const(-1)])"),
        "unbounded() must remain an explicit independent spelling");
    assert_eq!(CHECKER.matches("self.rt(\"__chan_new_bounded\", vec![n])").count(), 2,
        "both Channel<T>.bounded and channel<T>(capacity:) must be checked");
    assert!(LIB.contains("fnc __chan_new_bounded(int cap): int {"));
    assert!(LIB.contains("if cap < 0 then"));
    assert!(LIB.contains("if cap < -1 then"));
    assert!(LIB.contains("__panic(14);"));
}

#[test]
fn negative_bounded_capacity_compiles_into_guest_denial_not_an_unbounded_queue() {
    // These are compiler-to-Wasm checks. The expect-panic=14 fixtures
    // must ALSO execute in the browser harness before runtime PASS.
    for (label, src) in [
        ("bounded-minus-one", INVALID_SENTINEL),
        ("bounded-minus-two", INVALID_OTHER),
        ("legacy-bounded-minus-one", INVALID_LEGACY),
        ("explicit-unbounded", VALID_UNBOUNDED),
    ] {
        let wasm = atto_compiler::v2::compile(atto_compiler::lexer::lex(src)
            .unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .unwrap_or_else(|e| panic!("{label}: compiler: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),
            "{label}: compiler did not emit a guest Wasm module");
    }
}

#[test]
fn rendezvous_readers_register_pending_futures_in_native_atto_source() {
    // Source-level regression: a cap=0 producer can make progress only if
    // readch / for-await advertises a pending read in the guest queue.
    for function in ["__chan_recv", "__chan_recv_user_opt"] {
        let head = format!("suspends fnc {function}(int c): int {{");
        let start = LIB.find(&head).unwrap_or_else(|| panic!("missing {function}"));
        let next = &LIB[start + head.len()..];
        let end = next.find("\n}\n").unwrap_or_else(|| panic!("unterminated {function}"));
        let source = &next[..end];
        assert!(source.contains("if __load(c, 0) == 0 then"),
            "{function}: must special-case a zero-capacity queue");
        assert!(source.contains("val pending = __chan_nb(c);"),
            "{function}: must register a waiting reader");
        assert!(source.contains("val ready = await __await(pending);"),
            "{function}: must cooperatively await the registered read");
        assert!(source.contains("if __load(c, 56) != 0 then __panic(10); fi"));
        assert!(source.contains("if __load(c, 64) != 0 then __panic(10); fi"));
    }
    assert!(LIB.contains("if ready == 0 then __panic(9); fi"),
        "direct readch must report closed/drained rendezvous as a closed read");
}

#[test]
fn rendezvous_guest_programs_compile_to_wasm() {
    // This compiles real guest sources but does NOT execute the Wasm.
    // The fixture expected stdout/exit contracts require a stepful browser
    // runner before they can be called runtime-verified.
    for (label, src) in [
        ("direct readch", RENDEZVOUS_DIRECT),
        ("for await", RENDEZVOUS_ITER),
        ("close wakes read", RENDEZVOUS_CLOSE),
    ] {
        let wasm = atto_compiler::v2::compile(atto_compiler::lexer::lex(src)
            .unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .unwrap_or_else(|e| panic!("{label}: compiler: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),
            "{label}: compiler did not emit a Wasm guest");
    }
}

#[test]
fn legacy_pipe_admission_denies_preexisting_public_readers_and_buffered_data() {
    // The typed atomic 3-port PipeHandle already rejects both conditions.
    // The legacy 2-port paths must not bypass the same ownership boundary.
    for function in ["__actor_pipe_channel", "__channel_pipe_actor"] {
        let head = format!("fnc {function}(");
        let start = PRELUDE.find(&head).unwrap_or_else(|| panic!("missing {function}"));
        let next = &PRELUDE[start..];
        let end = next.find("\n}\n").unwrap_or_else(|| panic!("unterminated {function}"));
        let source = &next[..end];
        assert!(source.contains("if __load(ch, 8) != 0 then __panic(10); fi"),
            "{function}: cannot redirect previously buffered public DATA");
        assert!(source.contains("if __load(ch, 40) != 0 then __panic(10); fi"),
            "{function}: cannot strand or steal pending public reader Futures");
    }
}

#[test]
fn legacy_pipe_ownership_denials_compile_to_real_guest_wasm() {
    // Only verifies compiler lowering; the Node gate must EXECUTE these
    // separate guests and observe panic_code=10 with empty stdout.
    for (label, src) in [
        ("producer pending public reader", LEGACY_PENDING_PRODUCER),
        ("consumer pending public reader", LEGACY_PENDING_CONSUMER),
        ("producer with buffered data", LEGACY_PRELOADED_PRODUCER),
        ("consumer with buffered data", LEGACY_PRELOADED_CONSUMER),
    ] {
        let wasm = atto_compiler::v2::compile(atto_compiler::lexer::lex(src)
            .unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .unwrap_or_else(|e| panic!("{label}: compiler: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),
            "{label}: compiler did not emit Wasm");
    }
}
