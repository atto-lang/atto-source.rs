//! Checked I/O result patterns: discard and unit-Ok are not arbitrary binds.
use atto_compiler::compile;

const MAIN:&str = "pub fnc main(): int { return 0; }";
fn src(arm:&str)->String {
    format!(r#"
        pub susp fnc check_result(Channel<int> c): int {{
            match c.try_send(42) over
              {arm}
              on Err(_) -> {{ return 7; }}
            end
        }}
        {MAIN}
    "#)
}

#[test]
fn unit_result_accepts_bare_ok_or_explicit_discard() {
    for ok in ["on Ok -> { return 42; }",
               "on Ok(_) -> { return 42; }"] {
        let binary=compile(&src(ok)).unwrap_or_else(|d|panic!("{ok}: {d}"));
        assert!(binary.starts_with(b"\0asm\x01\0\0\0"), "{ok}");
    }
}

#[test]
fn explicit_discard_remains_valid_for_payload_results() {
    let binary=compile(&format!(r#"
        pub susp fnc check_result(Channel<int> c): int {{
            match << c over
              on Ok(_) -> {{ return 42; }}
              on Err(_) -> {{ return 7; }}
            end
        }}
        {MAIN}
    "#)).expect("a Result<Option<int>,IoError> Ok(_) may discard its payload");
    assert!(binary.windows("channel_int_await_read_v1".len())
        .any(|w|w==b"channel_int_await_read_v1"));
}

#[test]
fn nested_match_of_unary_read_after_successful_write_compiles() {
    let code=format!(r#"
        pub susp fnc check_result(Channel<int> c): int {{
          match c.try_send(42) over
            on Ok(_) -> {{
              match << c over
                on Ok(option) -> {{
                  match option over
                    on Some(value) -> {{ return value; }}
                    on None -> {{ return 11; }}
                  end
                }}
                on Err(_) -> {{ return 14; }}
              end
            }}
            on Err(_) -> {{ return 10; }}
          end
        }}
        {MAIN}
    "#);
    let binary=compile(&code).expect("on Ok(_) -> match << c must preserve a checked read");
    assert!(binary.windows("channel_int_await_read_v1".len())
        .any(|w|w==b"channel_int_await_read_v1"));
}

#[test]
fn named_void_ok_payload_and_missing_value_payload_fail_closed() {
    for (code,expected) in [
        (src("on Ok(value) -> { return value; }"),"E_ARITY"),
        (format!(r#"pub susp fnc check_result(Channel<int> c): int {{
            match readch c over
              on Ok -> {{ return 0; }}
              on Err(_) -> {{ return 1; }}
            end
        }}
        {MAIN}"#),"E_ARITY"),
    ] {
        let failure=compile(&code).expect_err("invalid Result pattern must not emit Wasm");
        assert_eq!(failure.code,expected,"{failure}");
    }
}
