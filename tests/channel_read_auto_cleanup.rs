//! Regression gate for compiler-generated Channel<int> read Future cleanup.
//! These are actual source-to-Wasm tests, not host-only reference oracles.
use atto_compiler::compile;

pub const SCOPED: &str = r#"
pub susp fnc main(): int {
  val opened = Channel<int>.bounded(0);
  match opened over
    on Err(error) -> { return -1; }
    on Ok(channel) -> {
      block {
        val pending = nb readch channel;
      }
      val closed = channel.close();
      val released = channel.release();
      match released over
        on Err(error) -> { return -2; }
        on Ok -> { return 42; }
      end
    }
  end
}
"#;

pub const COMMITTED: &str = r#"
pub susp fnc main(): int {
  val opened = Channel<int>.bounded(0);
  match opened over
    on Err(error) -> { return -1; }
    on Ok(channel) -> {
      block {
        val pending = nb readch channel;
        val sent = channel.send(41);
      }
      val second = nb readch channel;
      val outcome = await second;
      match outcome over
        on Err(error) -> { return -2; }
        on Ok(item) -> {
          match item over
            on None -> { return -3; }
            on Some(value) -> { return value + 1; }
          end
        }
      end
    }
  end
}
"#;

pub const BREAK: &str = r#"
pub susp fnc main(): int {
  val opened = Channel<int>.bounded(0);
  match opened over
    on Err(error) -> { return -1; }
    on Ok(channel) -> {
      loop {
        val pending = nb readch channel;
        break;
      }
      val closed = channel.close();
      val released = channel.release();
      match released over
        on Err(error) -> { return -2; }
        on Ok -> { return 42; }
      end
    }
  end
}
"#;

#[test]
fn lexical_early_exit_and_committed_rehoming_emit_guest_abandon_import() {
    for source in [SCOPED, COMMITTED, BREAK] {
        let wasm = compile(source).unwrap_or_else(|e| panic!("cleanup source must compile: {e}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        assert!(wasm.windows(b"channel_int_abandon_v1".len())
            .any(|w| w == b"channel_int_abandon_v1"), "no compiler-generated abandon ABI");
        assert_eq!(wasm, compile(source).expect("deterministic cleanup lowering"));
    }
}

#[test]
fn unowned_read_ticket_discard_and_overwrite_fail_closed() {
    let discard = SCOPED.replace("val pending = nb readch channel;", "nb readch channel;");
    let error = compile(&discard).expect_err("bare read must not lose its ticket");
    assert_eq!(error.code,"E_CHANNEL_OWNERSHIP","{error}");
    let overwrite = SCOPED.replace("val pending = nb readch channel;",
        "let pending = nb readch channel; pending = nb readch channel;");
    let error = compile(&overwrite).expect_err("overwrite must not lose a pending read");
    assert_eq!(error.code,"E_CHANNEL_OWNERSHIP","{error}");
}
