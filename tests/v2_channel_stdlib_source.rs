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
const TRY_OPEN_DRAIN: &str =
    include_str!("fixtures/pass/245-v2-try-read-open-empty-and-closed-drain.atto");
const TRY_CLOSED_EMPTY: &str =
    include_str!("fixtures/panic/245-v2-try-read-closed-empty-denial.atto");
const TRY_CLOSED_DRAINED: &str =
    include_str!("fixtures/panic/246-v2-try-read-after-drain-denial.atto");
const CANCEL_FIFO_TAIL: &str =
    include_str!("fixtures/pass/247-v2-cancel-read-fifo-tail.atto");
const CANCEL_WRONG_CHANNEL: &str =
    include_str!("fixtures/pass/248-v2-cancel-read-wrong-channel.atto");
const CANCEL_DELIVERED: &str =
    include_str!("fixtures/pass/249-v2-cancel-read-after-delivery.atto");
const CANCEL_CLOSED: &str =
    include_str!("fixtures/pass/250-v2-cancel-read-after-close.atto");
const CANCEL_AWAIT_TRAPS: &str =
    include_str!("fixtures/panic/247-v2-cancel-read-await-traps.atto");
const CANCEL_WRONG_TYPE: &str =
    include_str!("fixtures/fail/247-v2-cancel-read-wrong-payload-type.atto");
const CANCEL_HEAD: &str =
    include_str!("fixtures/pass/251-v2-cancel-read-head-preserves-fifo.atto");
const CANCEL_MIDDLE: &str =
    include_str!("fixtures/pass/252-v2-cancel-read-middle-preserves-fifo.atto");
const CANCEL_FORGED: &str =
    include_str!("fixtures/fail/248-v2-cancel-read-nonfuture-denial.atto");
const CANCEL_GRAPH_ADMISSION: &str =
    include_str!("fixtures/pass/253-v2-cancel-read-releases-graph-admission.atto");
const WRITE_CANCEL_RENDEZVOUS: &str =
    include_str!("fixtures/pass/254-v2-pending-write-cancel-rendezvous.atto");
const WRITE_CANCEL_FULL: &str =
    include_str!("fixtures/pass/255-v2-pending-write-cancel-full-buffer.atto");
const WRITE_ADMIT_AFTER_READ: &str =
    include_str!("fixtures/pass/256-v2-pending-write-admission-after-read.atto");
const WRITE_NB_READ: &str =
    include_str!("fixtures/pass/257-v2-pending-write-rendezvous-nb-read.atto");
const WRITE_TRY_READ: &str =
    include_str!("fixtures/pass/258-v2-pending-write-try-rendezvous.atto");
const WRITE_CLOSE_RECOVERY: &str =
    include_str!("fixtures/pass/259-v2-pending-write-close-recovery.atto");
const WRITE_ALREADY_CLOSED: &str =
    include_str!("fixtures/pass/260-v2-pending-write-already-closed-recovery.atto");
const WRITE_WRONG_CHANNEL: &str =
    include_str!("fixtures/pass/261-v2-pending-write-wrong-channel.atto");
const WRITE_FIFO: &str =
    include_str!("fixtures/pass/262-v2-pending-write-fifo-rendezvous.atto");
const WRITE_CANCEL_AWAIT: &str =
    include_str!("fixtures/panic/263-v2-pending-write-cancelled-await.atto");
const WRITE_CLOSED_AWAIT: &str =
    include_str!("fixtures/panic/264-v2-pending-write-closed-await.atto");
const BORROW_NESTED_CHANNEL: &str =
    include_str!("fixtures/fail/263-v2-borrow-channel-nested-capability.atto");
const BORROW_ANY: &str =
    include_str!("fixtures/fail/264-v2-borrow-channel-any-erasure.atto");
const BORROW_MUTABLE_LIST: &str =
    include_str!("fixtures/fail/265-v2-borrow-channel-mutable-list.atto");
const BORROW_STRING_PENDING: &str =
    include_str!("fixtures/fail/266-v2-borrow-pending-write-string.atto");
const BORROW_WRONG_WRITE_TICKET: &str =
    include_str!("fixtures/fail/267-v2-borrow-pending-write-wrong-ticket.atto");
const BORROW_FORGED_WRITE_TICKET: &str =
    include_str!("fixtures/fail/268-v2-borrow-pending-write-forged-ticket.atto");


#[test]
fn channel_module_is_extracted_once_and_not_duplicated_in_prelude() {
    assert_eq!(PRELUDE.matches(SLOT).count(), 1);
    assert!(!PRELUDE.contains("fnc __chan_new("));
    assert!(!PRELUDE.contains("fnc __chan_send("));
    let declarations = [
        "__chan_new", "__chan_new_bounded", "__chan_offer", "__chan_send", "__chan_send_pipe",
        "__chan_offer_ready", "__chan_drain_writers", "__chan_send_pending",
        "__chan_cancel_write", "__chan_take", "__chan_recv", "__chan_recv_user_opt",
        "__chan_recv_opt", "__chan_try", "__chan_nb", "__chan_cancel_read",
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

#[test]
fn immediate_read_distinguishes_open_empty_from_closed_drained() {
    let start = LIB.find("fnc __chan_try(int c): int {").expect("missing try reader");
    let next = &LIB[start..];
    let end = next.find("\n}\n").expect("unterminated try reader");
    let source = &next[..end];
    let buffered = source.find("if __load(c, 8) > 0 then")
        .expect("must drain buffered FIFO before reporting closed");
    let closed = source.find("if __load(c, 32) != 0 then __panic(9); fi")
        .expect("closed and drained is not temporarily unready None");
    let empty = source.find("return 0;").expect("open+empty remains None");
    assert!(buffered < closed && closed < empty,
        "closed try-read must drain committed data before terminal denial");
}

#[test]
fn terminal_try_read_guest_cases_compile_to_real_wasm() {
    // Compile only; actual Wasm stdout/exit/panic assertions are in the
    // timeout-bounded Node guest harness and need stepful CI execution.
    for (label, src) in [
        ("open empty, buffered drain", TRY_OPEN_DRAIN),
        ("closed empty", TRY_CLOSED_EMPTY),
        ("closed after final drain", TRY_CLOSED_DRAINED),
    ] {
        let wasm = atto_compiler::v2::compile(atto_compiler::lexer::lex(src)
            .unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .unwrap_or_else(|e| panic!("{label}: compiler: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),
            "{label}: compiler did not emit Wasm");
    }
}

#[test]
fn cancelled_read_is_atomically_unlinked_and_not_reported_as_eof() {
    let start = LIB.find("fnc __chan_cancel_read(int c, int f): int {")
        .expect("explicit cancellation is native Atto source");
    let next = &LIB[start..];
    let end = next.find("\n}\n").expect("unterminated cancellation helper");
    let code = &next[..end];
    assert!(code.contains("if __load(c, 56) != 0 then __panic(10); fi"));
    assert!(code.contains("if __load(c, 64) != 0 then __panic(10); fi"));
    assert!(code.contains("if __load(node, 0) == f then"));
    assert!(code.contains("__store(c, 40, next);"));
    assert!(code.contains("__store(prev, 8, next);"));
    assert!(code.contains("if __load(c, 48) == node then"));
    assert!(code.contains("__store(c, 48, prev);"));
    assert!(code.contains("__store(f, 0, 2);"));
    assert!(code.contains("return 1;"));
    assert!(code.contains("return 0;"));
    assert!(PRELUDE.contains("if __load(f, 0) == 2 then __panic(15); fi"),
        "cancelled Future<Option<T>> must not masquerade as normal None EOF");
    assert!(CHECKER.contains("\"cancel_read\" => {"));
    assert!(CHECKER.contains("Ty::Future(Box::new(Ty::Option(item.clone())))"),
        "cancel_read requires a typed Future<Option<T>> for this Channel<T>");
    assert!(CHECKER.contains("self.rt(\"__chan_cancel_read\", vec![recv, pending])"));
}

#[test]
fn cancellation_guest_fixtures_compile_and_bad_future_type_fails_closed() {
    // Compiled-guest execution is separately required in the Node gate.
    for (label, src) in [
        ("cancel pending tail and preserve FIFO", CANCEL_FIFO_TAIL),
        ("wrong channel cannot revoke reader", CANCEL_WRONG_CHANNEL),
        ("delivered Future cannot be cancelled", CANCEL_DELIVERED),
        ("close completion cannot be revoked", CANCEL_CLOSED),
        ("await cancelled Future is panic 15", CANCEL_AWAIT_TRAPS),
        ("cancel head preserves FIFO", CANCEL_HEAD),
        ("cancel middle preserves FIFO", CANCEL_MIDDLE),
        ("cancel frees checked DATA graph admission", CANCEL_GRAPH_ADMISSION),
    ] {
        let wasm = atto_compiler::v2::compile(atto_compiler::lexer::lex(src)
            .unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .unwrap_or_else(|e| panic!("{label}: compiler: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),
            "{label}: compiler did not emit Wasm");
    }
    let invalid = atto_compiler::v2::compile(atto_compiler::lexer::lex(CANCEL_WRONG_TYPE)
        .expect("type-negative source lexes")).expect_err("wrong Future<Option<T>> must fail");
    assert_eq!(invalid.code, "E_TYPE",
        "wrong-element-type pending Future cannot revoke this channel's readers");
    let forged = atto_compiler::v2::compile(atto_compiler::lexer::lex(CANCEL_FORGED)
        .expect("forged-Future negative source lexes")).expect_err("integer is not a Future");
    assert_eq!(forged.code, "E_TYPE",
        "user numbers may not forge a cancellation registration capability");
}


#[test]
fn pending_write_ownership_and_borrow_gate_are_declared_in_guest_source() {
    assert!(LIB.contains("val c = __alloc(88);"),
        "write queue requires two additional words, not an overrun of old channel layout");
    assert!(LIB.contains("__q_push(c + 72, f);"),
        "an unadmitted pending writer must retain a registration");
    assert!(LIB.contains("if __load(c, 72) != 0 then return 0; fi"),
        "new direct send must not leapfrog an already-registered writer");
    assert!(LIB.contains("__chan_drain_writers(c);"),
        "freeing a bounded slot or registering a read must wake old writers");
    assert!(LIB.contains("__store(f, 0, 3);"),
        "closed writes must retain payload instead of reporting success");
    assert!(LIB.contains("if __load(c, 80) == node then"),
        "cancel_write must repair the pending queue's tail");
    assert!(LIB.contains("__store(f, 8, 0);"),
        "the future must not retain a second recoverable payload reference");
    assert!(LIB.contains("return __some(payload);"),
        "the old value must be returned on cancellation or closed rejection");
    assert!(PRELUDE.contains("if __load(f, 0) == 3 then __panic(8); fi"),
        "closed pending write must not resolve to successful None");
    assert!(PRELUDE.contains("if __load(f, 0) == 4 then __panic(15); fi"),
        "reclaimed closed ticket must remain revoked");
    for fn_name in ["__actor_pipe_channel", "__channel_pipe_actor", "__actor_channel_actor_try"] {
        let start = PRELUDE.find(&format!("fnc {fn_name}(")).unwrap();
        let end = PRELUDE[start..].find("\n}\n").unwrap() + start;
        assert!(PRELUDE[start..end].contains("if __load(ch, 72) != 0 then"),
            "{fn_name}: actor DATA may not claim queued public writes");
    }
    assert!(CHECKER.contains("fn channel_transport_safe(t: &Ty) -> bool"),
        "borrow boundary requires a data-only channel payload gate");
    assert!(CHECKER.contains("if !Self::channel_transport_safe(&payload)"),
        "core Channel<T> construction must enforce the borrow/transport gate");
    assert!(CHECKER.contains("send_pending requires copy-safe int/bool"),
        "pending writes must stay scalar-only without an affine move checker");
    assert!(CHECKER.contains("self.rt(\"__chan_send_pending\", vec![recv, payload])"));
    assert!(CHECKER.contains("self.rt(\"__chan_cancel_write\", vec![recv, ticket])"));
}

#[test]
fn pending_write_guests_compile_to_real_wasm_and_borrow_violations_fail() {
    // Only compilation evidence: runtime PASS additionally requires the
    // independent timeout-bounded node-v2-channel-guest.mjs execution gate.
    for (label, source) in [
        ("cancel uncommitted rendezvous write", WRITE_CANCEL_RENDEZVOUS),
        ("cancel full bounded write", WRITE_CANCEL_FULL),
        ("admit pending writer after drain", WRITE_ADMIT_AFTER_READ),
        ("nb readch rendezvous with pending writer", WRITE_NB_READ),
        ("try readch rendezvous with pending writer", WRITE_TRY_READ),
        ("recover pending payload after close", WRITE_CLOSE_RECOVERY),
        ("recover send attempted on closed channel", WRITE_ALREADY_CLOSED),
        ("wrong channel cannot cancel write", WRITE_WRONG_CHANNEL),
        ("FIFO pending writes under rendezvous", WRITE_FIFO),
        ("await cancelled writer traps", WRITE_CANCEL_AWAIT),
        ("await closed writer traps", WRITE_CLOSED_AWAIT),
    ] {
        let wasm = atto_compiler::v2::compile(
            atto_compiler::lexer::lex(source).unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .unwrap_or_else(|e| panic!("{label}: compiler: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"),
            "{label}: compiler failed to produce actual Wasm");
    }
    for (label, source, want) in [
        ("nested channel capability transport", BORROW_NESTED_CHANNEL, "E_BORROW"),
        ("Any dynamic payload transport", BORROW_ANY, "E_BORROW"),
        ("mutable list transport", BORROW_MUTABLE_LIST, "E_BORROW"),
        ("pending string write without move checking", BORROW_STRING_PENDING, "E_BORROW"),
        ("wrong element type write ticket", BORROW_WRONG_WRITE_TICKET, "E_TYPE"),
        ("forged integer write ticket", BORROW_FORGED_WRITE_TICKET, "E_TYPE"),
    ] {
        let failure = atto_compiler::v2::compile(
            atto_compiler::lexer::lex(source).unwrap_or_else(|e| panic!("{label}: lexer: {e}")))
            .expect_err(&format!("{label}: compiler accepted an unsafe channel program"));
        assert_eq!(failure.code, want, "{label}: wrong deterministic compiler diagnostic");
    }
}
