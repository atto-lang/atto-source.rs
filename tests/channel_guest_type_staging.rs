//! Stage-1 typed, checked Channel<int> ADT profiles.
//! No source-level channel imports are allowed until JSPI lowering exists.
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
fn typed_channel_future_read_still_rejects_unsupported_lowering() {
    for src in [
        r#"pub susp fnc main(): int { val pending: Future<Result<Option<int>,IoError>> = nb readch inbox; return await 42; }"#,
        r#"pub susp fnc main(): int { val ch: Channel<int> = 42; return 0; }"#,
        r#"pub susp fnc main(): int { val pending: Future<Result<Option<int>,string>> = task { return 42; }; return 0; }"#,
    ] {
        let error=compile(src).expect_err("unsupported guest reads must not emit Wasm");
        assert!(matches!(error.code,"E_UNSUPPORTED"|"E_UNSUPPORTED_TYPE"|"E_TYPE"),"{src}: {error}");
    }
}
