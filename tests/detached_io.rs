use atto_compiler::detached_io::{inspect, Callback, Kind};
use atto_compiler::io_shorthand::{inspect as inspect_direction, Context, Direction, Mode};
use atto_compiler::lexer::lex;

fn parse(source: &str) -> Vec<atto_compiler::detached_io::DetachedIo> {
    inspect(&lex(source).unwrap()).unwrap()
}
fn rejected(source: &str) -> &'static str {
    inspect(&lex(source).unwrap()).unwrap_err().code
}

#[test]
fn accepts_supervised_detached_writes_without_callback() {
    let writes = parse("do writech foo, \"hi\"; do writech stdio.out, payload;");
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].kind, Kind::Write);
    assert_eq!(writes[0].handle, "foo");
    assert_eq!(writes[1].handle, "stdio.out");
    assert_eq!(writes[0].callback, Callback::None);
}
#[test]
fn accepts_success_only_callback_on_write() {
    let writes = parse("do writech foo, \"hi\", || -> void { log_success(); };");
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].callback, Callback::SuccessOnly);
}
#[test]
fn accepts_typed_outcome_callbacks_on_reads_and_writes() {
    let src = r#"
do readch inbox, |outcome| -> void { handle_read(outcome); };
do writech outgoing, f(1, 2), |receipt| -> void { handle_write(receipt); };
"#;
    let operations = parse(src);
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].kind, Kind::Read);
    assert_eq!(operations[0].callback, Callback::Outcome("outcome".to_string()));
    assert_eq!(operations[1].kind, Kind::Write);
    assert_eq!(operations[1].callback, Callback::Outcome("receipt".to_string()));
}
#[test]
fn nested_lambda_blocks_and_optional_terminal_semicolon() {
    let operations = parse("do writech foo, \"hi\", |result| -> void { if ready then { log(result); } fi }");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].callback, Callback::Outcome("result".to_string()));
}
#[test]
fn bare_detached_reads_fail_without_any_consuming_effect() {
    for src in [
        "do readch inbox;",
        "do readch inbox",
        "do readch stdio.in;",
        "do readch inbox, || -> void {};",
    ] {
        assert_eq!(rejected(src), "E_IO_DISCARD", "{src}");
    }
}
#[test]
fn malformed_callbacks_fail_closed() {
    for src in [
        "do readch foo, |msg| -> string {};",
        "do readch foo, |msg| -> void {",
        "do readch foo, |msg, error| -> void {};",
        "do readch foo, |return| -> void {};",
        "do writech foo, \"hi\", |result| -> int {};",
        "do writech foo, \"hi\", |result| -> void {",
        "do writech foo, \"hi\", value;",
    ] {
        assert_eq!(rejected(src), "E_IO_CALLBACK", "{src}");
    }
}
#[test]
fn malformed_write_payloads_and_handles_are_rejected() {
    for src in [
        "do writech foo;",
        "do writech foo, ;",
        "do writech foo, ",
        "do writech foo(), \"hi\";",
        "do readch foo(), |outcome| -> void {};",
    ] {
        assert_eq!(rejected(src), "E_IO_SYNTAX", "{src}");
    }
}
#[test]
fn direction_operator_do_write_is_detached_not_direct_stdout() {
    let parsed = inspect_direction(&lex("do stdio.out << \"hi\";").unwrap()).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].mode, Mode::Detached);
    assert_eq!(parsed[0].context, Context::Standalone);
    assert_eq!(parsed[0].direction, Direction::Write);
}
#[test]
fn direction_operator_do_read_never_discards_a_local_binding() {
    for src in ["do inbox >> val message;", "do val message << inbox;", "do stdio.in >> val line;"] {
        let err = inspect_direction(&lex(src).unwrap()).unwrap_err();
        assert_eq!(err.code, "E_IO_DISCARD", "{src}: {err}");
    }
}
#[test]
fn existing_do_select_and_do_match_have_no_detached_io() {
    for src in [
        "do select { when inbox >> val x -> {} }",
        "do nb select { when outgoing << \"hi\" -> {} }",
        "do match value over _ -> {} end",
        "val s = \"do writech foo, hi\"; // do readch foo",
    ] {
        assert!(parse(src).is_empty(), "{src}");
    }
}
#[test]
fn compiler_never_lowers_detached_io_into_sync_stdout_guest() {
    for src in [
        "pub fnc main(): void { do writech stdio.out, \"hi\"; }",
        "pub fnc main(): void { do stdio.out << \"hi\"; }",
        "pub fnc main(): void { do readch stdio.in, |outcome| -> void {}; }",
        "pub fnc main(): void { do writech foo, \"hi\", || -> void {}; }",
    ] {
        let err = atto_compiler::compile(src).expect_err("detached runtime is not implemented");
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}: {err}");
    }
}
#[test]
fn valid_direct_stdout_still_compiles_and_cannot_be_mistaken_for_do() {
    let baseline = "pub fnc main(): int { stdio.out << \"hi\"; return 7; }";
    let wasm = atto_compiler::compile(baseline).expect("direct stdout is supported");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
}
