//! The canonical checked stdin echo must compile from real Atto source.
//! This is a *compiler unit* test, not a guest runtime execution receipt.
use atto_compiler::compile;

const ECHO: &str = r#"pub suspends fnc main(): int using io, suspend {
  val input = stdio.in;
  val output = process.stdout;
  val prompt = writech output, "Echo the next input item:";
  return match prompt over
    on Err(_) -> 10,
    on Ok(_) -> match << input over
      on Err(_) -> 14,
      on Ok(line) -> match line over
        on None -> 11,
        on Some(item) -> match writech output, item over
          on Err(_) -> 12,
          on Ok(_) -> match writech stdio.err, "echo complete" over
            on Ok(_) -> 0,
            on Err(_) -> 13,
          end,
        end,
      end,
    end,
  end;
}"#;
fn includes(wasm:&[u8],token:&str)->bool {
    wasm.windows(token.len()).any(|w|w==token.as_bytes())
}

#[test]
fn nested_return_match_unary_stdin_is_typed_and_abi_marked() {
    let bytes=compile(ECHO).expect("real typed stdin echo compiles");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    for symbol in ["atto.stdio_text","atto.async","stdio_text_read_v1",
                   "stdio_text_write_v1","cooperate"] {
        assert!(includes(&bytes,symbol),"checked stdin echo omitted {symbol}");
    }
}

#[test]
fn readch_and_unary_prefix_compile_to_identical_checked_abi() {
    let other=ECHO.replace("match << input over", "match readch input over");
    let prefix=compile(ECHO).expect("prefix stdin read");
    let explicit=compile(&other).expect("readch stdin read");
    assert_eq!(prefix,explicit,"two spellings must not diverge in JSPI lowering");
}

#[test]
fn all_four_nested_checked_stdin_grammars_compile_to_versioned_guest_abi() {
    let nominal = ECHO
        .replace("on Err(_)", "on is Err<error>")
        .replace("on Ok(_)", "on is Ok<_>")
        .replace("on Ok(line)", "on is Ok<line>")
        .replace("on Some(item)", "on is Some<item>")
        .replace("on None", "on is None");
    // Check every combination rather than assuming frontend acceptance
    // implies generated guest execution or API parity.
    for (name, source) in [
        ("paren + unary", ECHO.to_owned()),
        ("paren + readch", ECHO.replace("match << input over", "match readch input over")),
        ("is ADT + unary", nominal.clone()),
        ("is ADT + readch", nominal.replace("match << input over", "match readch input over")),
    ] {
        let wasm=compile(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(wasm.starts_with(b"\\0asm\\x01\\0\\0\\0"), "{name}");
        for import in ["atto.stdio_text", "stdio_text_read_v1",
                       "stdio_text_write_v1", "atto.async"] {
            assert!(includes(&wasm,import), "{name}: missing {import}");
        }
    }
}

#[test]
fn process_stdin_aliases_and_explicit_type_are_statically_checked() {
    let sample=r#"
        pub suspends fnc main(): int using io, suspend {
          val Channel<string> input = process.stdin;
          val result: Result<Option<string>,IoError> = << input;
          return match result over
            on Ok(_) -> 0,
            on Err(_) -> 1,
          end;
        }
    "#;
    let binary=compile(sample).expect("typed stdin aliases compile");
    assert!(includes(&binary,"stdio_text_read_v1"));
}

#[test]
fn invalid_direction_effect_and_unchecked_result_never_emit_guest() {
    for (source,code) in [
        (r#"pub fnc main(): int { val input=stdio.in; val result=<< input; return 0; }"#,"E_SUSPENDS_REQUIRED"),
        (r#"pub susp fnc main(): int { val c=stdio.out; val result=readch c; return 0; }"#,"E_IO_READABLE"),
        (r#"pub susp fnc main(): int { val c=stdio.in; val result=writech c, "bad"; return 0; }"#,"E_IO_WRITABLE"),
        (r#"pub susp fnc main(): int { readch stdio.in; return 0; }"#,"E_IO_DISCARD"),
        (r#"pub susp fnc main(): int { val result=readch stdio.in; val string line=result; return 0; }"#,"E_TYPE"),
        (r#"pub susp fnc main(): int { val item=readch stdio.in; val status=writech stdio.out, item; return 0; }"#,"E_IO_PAYLOAD_TYPE"),
        (r#"pub susp fnc main(): int { val c=new Channel<string>(1); return 0; }"#,"E_UNSUPPORTED"),
    ] {
        let err=compile(source).expect_err("unsupported or unsafe stdio guest must fail closed");
        assert_eq!(err.code,code,"{source}: {err}");
    }
}
