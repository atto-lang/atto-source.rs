//! Real compiler source-to-guest Channel<int> import and checked types.
use atto_compiler::compile;
const SRC: &str = r#"
pub susp fnc main(): int {
  val opened: Result<Channel<int>,IoError> = Channel<int>.bounded(0);
  match opened over
    on Err(error) -> { return -3; }
    on Ok(channel) -> {
      val pending: Future<Result<Option<int>,IoError>> = nb readch channel;
      val sent: Result<void,IoError> = channel.send(41);
      match sent over
        on Err(error) -> { return -2; }
        on Ok -> {
          val result: Result<Option<int>,IoError> = await pending;
          match result over
            on Err(error) -> { return -1; }
            on Ok(item) -> {
              match item over
                on None -> { return 0; }
                on Some(value) -> { return value + 1; }
              end
            }
          end
        }
      end
    }
  end
}
"#;
fn has(h:&[u8],n:&[u8])->bool{h.windows(n.len()).any(|w| w==n)}
#[test]
fn compiled_guest_channel_imports_are_versioned_and_deterministic() {
 let wasm=compile(SRC).expect("Channel<int> guest source compiles");
 assert_eq!(&wasm[..8],b"\0asm\x01\0\0\0");
 for name in ["atto.channel_int","atto.async","channel_int_open_v1",
    "channel_int_nb_read_v1","channel_int_send_v1",
    "channel_int_await_read_v1","cooperate"] {
   assert!(has(&wasm,name.as_bytes()),"missing {name}");
 }
 assert_eq!(wasm,compile(SRC).unwrap());
}
#[test]
fn nested_caret_and_return_match_are_identical_compiled_guest_control_flow() {
    // Exact user-requested control flow, including nested nonlocal returns
    // and a suspended Channel<int> future. No implicit unwrap is introduced.
    let caret = SRC
        .replace("  match opened over", "  ^ match opened over")
        .replace("      match sent over", "      ^ match sent over")
        .replace("          match result over", "          ^ match result over")
        .replace("              match item over", "              ^ match item over");
    assert_eq!(caret.matches("^ match").count(), 4);
    let explicit_return = caret.replace("^ match", "return match");
    let caret_wasm = compile(&caret)
        .unwrap_or_else(|e| panic!("nested caret match source rejected: {e}"));
    let return_wasm = compile(&explicit_return)
        .unwrap_or_else(|e| panic!("nested return match source rejected: {e}"));
    assert_eq!(caret_wasm, return_wasm,
        "^ match and return match must lower to identical typed Wasm");
    for name in ["atto.channel_int","atto.async","channel_int_await_read_v1"] {
        assert!(has(&caret_wasm,name.as_bytes()),"missing compiled channel ABI {name}");
    }
}

#[test]
fn unsupported_channel_profiles_fail_closed() {
 for src in [
  r#"pub susp fnc main(): int { val ch=Channel<string>.bounded(1); return 0; }"#,
  r#"pub susp fnc main(): int { val ch=Channel<int>.bounded(2048); return 0; }"#,
  r#"pub fnc main(): int { val f=nb readch 4; return 0; }"#,
  r#"pub susp fnc main(): int { val f=nb readch 4; return 0; }"#,
 ] {
  assert!(compile(src).is_err(),"{src} emitted unsupported guest Wasm");
 }
}

const CANCEL_SOURCE: &str = r#"
pub susp fnc main(): int {
    val opened = Channel<int>.bounded(0);
    match opened over
        on Err(error) -> { return -1; }
        on Ok(channel) -> {
            val pending = nb readch channel;
            val cancelled: Result<void,IoError> = pending.cancel();
            match cancelled over
                on Err(error) -> { return -2; }
                on Ok -> {
                    val outcome: Result<Option<int>,IoError> = await pending;
                    match outcome over
                        on Err(error) -> { return 42; }
                        on Ok(item) -> { return -3; }
                    end
                }
            end
        }
    end
}
"#;
const ABANDON_SOURCE: &str = r#"
pub susp fnc main(): int {
    val opened = Channel<int>.bounded(0);
    match opened over
        on Err(error) -> { return -1; }
        on Ok(channel) -> {
            val pending = nb readch channel;
            val sent: Result<void,IoError> = channel.send(41);
            match sent over
                on Err(error) -> { return -2; }
                on Ok -> {
                    val discarded: Result<void,IoError> = pending.abandon();
                    match discarded over
                        on Err(error) -> { return -3; }
                        on Ok -> {
                            val reread = nb readch channel;
                            val outcome = await reread;
                            match outcome over
                                on Err(error) -> { return -4; }
                                on Ok(item) -> {
                                    match item over
                                        on None -> { return -5; }
                                        on Some(n) -> { return n + 1; }
                                    end
                                }
                            end
                        }
                    end
                }
            end
        }
    end
}
"#;
#[test]
fn explicit_cancel_and_abandon_emit_only_checked_versioned_imports() {
    for (source, imported) in [
        (CANCEL_SOURCE, "channel_int_cancel_v1"),
        (ABANDON_SOURCE, "channel_int_abandon_v1"),
    ] {
        let wasm = compile(source).unwrap_or_else(|e| panic!("{imported}: {e}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        for name in ["atto.channel_int","atto.async","channel_int_nb_read_v1",imported] {
            assert!(has(&wasm,name.as_bytes()), "missing {name}");
        }
        assert_eq!(wasm, compile(source).expect("deterministic compiled channel lifecycle"));
    }
}
#[test]
fn use_after_explicit_abandon_never_emits_guest_wasm() {
    let invalid = ABANDON_SOURCE.replace(
        "val reread = nb readch channel;",
        "val reread = await pending;",
    );
    let error=compile(&invalid).expect_err("abandoned ticket must be a moved linear capability");
    assert_eq!(error.code,"E_MOVED","{error}");
}
