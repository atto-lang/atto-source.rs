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
