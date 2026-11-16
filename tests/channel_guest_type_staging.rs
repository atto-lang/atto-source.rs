//! Checked Channel<int> ADT profiles and negative source diagnostics.
//! Valid guest channel imports are covered by channel_guest_compiled_read.rs;
//! this suite keeps unbound/ill-typed sources fail-closed.
use atto_compiler::compile;

fn contains(bytes:&[u8],needle:&[u8])->bool {
    bytes.windows(needle.len()).any(|w| w==needle)
}
#[test]
fn checked_nested_channel_result_option_is_a_tagged_adt() {
    let src=r#"
pub fnc main(): int {
    val outcome: Result<Option<int>,IoError> = Ok(Some(41));
    match outcome over
        on Ok(item) -> {
            match item over
                on Some(value) -> { return value + 1; }
                on None -> { return 0; }
            end
        }
        on Err(error) -> { return -1; }
    end
}
"#;
    let wasm=compile(src).expect("checked nested Option/Result must compile");
    assert_eq!(&wasm[..8],b"\0asm\x01\0\0\0");
    assert!(!contains(&wasm,b"atto.channel_int"),"type-only code must not acquire channel capability");
    assert!(!contains(&wasm,b"channel_int_"),"no guest channel import admitted");
}
#[test]
fn channel_read_rejects_unbound_receiver_and_invalid_types() {
    for (src, expected) in [
        (r#"pub susp fnc main(): int { val pending: Future<Result<Option<int>,IoError>> = nb readch inbox; return await 42; }"#, "E_UNBOUND"),
        (r#"pub susp fnc main(): int { val ch: Channel<int> = 42; return 0; }"#, "E_TYPE"),
        (r#"pub susp fnc main(): int { val pending: Future<Result<Option<int>,string>> = task { return 42; }; return 0; }"#, "E_UNSUPPORTED_TYPE"),
    ] {
        let error=compile(src).expect_err("invalid channel source must not emit guest Wasm");
        assert_eq!(error.code, expected, "{src}: {error}");
    }
}
