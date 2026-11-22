//! Continuous 'pipe' is a typed-transport contract, never a second
//! meaning of single-operation << / >> or functional |> / <|.
//! These are actual compiler fail-closed tests; the reference pump oracle
//! remains separate until the supervised transport guest ABI exists.
use atto_compiler::{compile, io_shorthand::{inspect, inspect_pipe, Direction}, lexer::lex};

#[test]
fn producer_pipe_consumer_recognizes_stream_and_actor_endpoints() {
    let src = r#"
        stdio.in pipe stdio.out;
        (input_channel) pipe (output_stream);
        spawn Producer() pipe output_channel;
        input_channel pipe spawn Consumer();
        spawn Source() pipe spawn Sink();
    "#;
    let items=inspect_pipe(&lex(src).unwrap()).unwrap();
    let pairs=items.iter().map(|p|(p.source.as_str(),p.sink.as_str())).collect::<Vec<_>>();
    assert_eq!(pairs,vec![
        ("stdio.in","stdio.out"),
        ("input_channel","output_stream"),
        ("spawn Producer()","output_channel"),
        ("input_channel","spawn Consumer()"),
        ("spawn Source()","spawn Sink()"),
    ]);
}

#[test]
fn continuous_pipe_fails_closed_before_guest_emission() {
    for operation in [
        "stdio.in pipe stdio.out;",
        "spawn Producer() pipe stdio.out;",
        "inbox pipe spawn Consumer();",
        "spawn Producer() pipe spawn Consumer();",
    ] {
        let src=format!("pub susp fnc main(): int {{ {operation} return 42; }}");
        let err=compile(&src).expect_err("guest transport not yet executable");
        assert_eq!(err.code,"E_UNSUPPORTED","{operation}: {err}");
        assert!(err.message.contains("pipe"),"{err}");
    }
}

#[test]
fn direction_operators_cannot_be_continuous_pump_or_existing_local_read() {
    for src in [
        "stdio.in >> stdio.out;",
        "reader >> writer;",
        "stdio.in >> 42;",
        "stdio.in >> stdio.out >> extra;",
        "(stdio.in) >> (stdio.out);",
    ] {
        let e=inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(e.code,"E_IO_SYNTAX","{src}: {e}");
    }
    assert!(compile("pub fnc main(): int { stdio.out << stdio.in; ^ 42; }").is_err(),
        "never stringify an input endpoint as one value or silently pump it");
}

#[test]
fn valid_single_reads_and_writes_are_not_keyword_pipes() {
    let src=r#"
        inbox >> val next: Result<Option<int>,IoError>;
        val response: Result<Option<int>,IoError> << inbox;
        stdio.out << 42;
    "#;
    let tokens=lex(src).unwrap();
    assert!(inspect_pipe(&tokens).unwrap().is_empty());
    let items=inspect(&tokens).unwrap();
    assert_eq!(items.len(),3);
    assert_eq!(items[0].direction,Direction::Read);
    assert_eq!(items[1].direction,Direction::Read);
    assert_eq!(items[2].direction,Direction::Write);
}

#[test]
fn malformed_pipe_chains_modes_guards_and_operands_fail_closed() {
    for src in [
        "nb source pipe sink;",
        "try source pipe sink;",
        "do source pipe sink;",
        "when source pipe sink -> {}",
        "source pipe sink pipe tail;",
        "source pipe ;",
        "source pipe sink",
        "source pipe sink << another;",
        "source pipe sink >> val received;",
        "^ source pipe sink;",
    ] {
        assert!(inspect_pipe(&lex(src).unwrap()).is_err(),"{src}");
    }
}

#[test]
fn strings_comments_and_function_pipelines_are_not_actor_pipes() {
    let src=r#"
        val literal = "x pipe y";
        // channel pipe sink;
        val value = 42 |> transform_result;
        val result = transform_result <| 41;
        stdio.out << "pipe";
    "#;
    assert!(inspect_pipe(&lex(src).unwrap()).unwrap().is_empty());
}

#[test]
fn pipe_rejects_fake_implicit_typed_stream_or_pipe_await() {
    for src in [
        "pub susp fnc main(): int { ^ stdio.in pipe stdio.out; }",
        "pub susp fnc main(): int { stdio.in pipe stdio.out; ^ 42; }",
    ] {
        let e=compile(src).expect_err("no unchecked guest pipe");
        assert!(matches!(e.code,"E_IO_SYNTAX"|"E_UNSUPPORTED"),"{e}");
    }
}
