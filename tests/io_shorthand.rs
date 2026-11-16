use atto_compiler::io_shorthand::{inspect, Context, Direction};
use atto_compiler::lexer::lex;

fn parse(input: &str) -> Vec<atto_compiler::io_shorthand::IoShorthand> {
    inspect(&lex(input).unwrap()).unwrap()
}

#[test]
fn recognizes_standalone_channel_and_stream_io() {
    let sample = r#"
stdio.out << "Hello";
stdio.in >> const line;
output_stream << payload;
input_stream >> let chunk;
"#;
    let ops = parse(sample);
    assert_eq!(ops.len(), 4, "{ops:?}");
    assert_eq!(ops[0].source_or_sink, "stdio.out");
    assert_eq!(ops[0].direction, Direction::Write);
    assert_eq!(ops[0].context, Context::Standalone);
    assert_eq!(ops[1].direction, Direction::Read);
    assert_eq!(ops[1].binder.as_deref(), Some("line"));
    assert_eq!(ops[2].source_or_sink, "output_stream");
    assert_eq!(ops[3].binder.as_deref(), Some("chunk"));
}

#[test]
fn recognizes_select_arms_and_body_writes() {
    let src = r#"
do select {
    when inbox >> const message -> {
        stdio.out << message;
    }
    when outgoing << "ready" -> {
        stdio.out << "Write completed";
    }
}
"#;
    let ops = parse(src);
    assert_eq!(ops.len(), 4, "{ops:?}");
    assert_eq!(ops[0].context, Context::SelectArm);
    assert_eq!(ops[0].binder.as_deref(), Some("message"));
    assert_eq!(ops[1].context, Context::Standalone);
    assert_eq!(ops[2].context, Context::SelectArm);
    assert_eq!(ops[2].direction, Direction::Write);
    assert_eq!(ops[2].source_or_sink, "outgoing");
    assert_eq!(ops[3].source_or_sink, "stdio.out");
}

#[test]
fn optional_read_type_and_val_mode_are_retained() {
    let ops = parse("inbox >> val x: int;");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].binder.as_deref(), Some("x"));
}

#[test]
fn comments_strings_and_nested_generic_closers_are_not_io() {
    let src = r#"
// outgoing << "not executed"
val text = "inbox >> const x";
val type_description = "Result<Option<int>>";
"#;
    assert!(parse(src).is_empty());
    assert!(parse("pub fnc main(): int { val a: int = 2; return a > > 1; }").is_empty());
    assert!(parse("pub fnc main(): int { val a: int = 2; return a << 1; }").is_empty());
}

#[test]
fn spaced_directions_do_not_match() {
    assert!(parse("out < < value;").is_empty());
    assert!(parse("input > > const value;").is_empty());
}

#[test]
fn malformed_io_shorthand_diagnostics() {
    for src in [
        "inbox >> message;",
        "inbox >> const ;",
        "outgoing << ;",
        "outgoing << value << other;",
        "do select { when inbox >> message -> {} }",
        "do select { when outgoing << -> {} }",
        "do select { when incoming >> const -> {} }",
    ] {
        let diag = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(diag.code, "E_IO_SYNTAX", "{src}: {diag}");
    }
}

#[test]
fn compiler_fail_closed_for_recognized_io_with_precise_position() {
    let src = "pub fnc main(): void {\n  stdio.out << \"Hello\";\n}";
    let err = atto_compiler::compile(src).expect_err("no channel/stream runtime in browser subset");
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert_eq!((err.pos.line, err.pos.col), (2, 13));
    assert!(err.message.contains("I/O << / >> shorthand parsed"));
}

#[test]
fn explicit_channel_forms_still_fail_closed_in_browser_subset() {
    for src in [
        "pub fnc main(): void { writech stdio.out, \"msg\"; }",
        "pub fnc main(): void { readch inbox; }",
    ] {
        let err = atto_compiler::compile(src).expect_err("browser subset does not implement channels");
        assert_eq!(err.code, "E_UNSUPPORTED");
    }
}

#[test]
fn read_annotation_requires_complete_nested_generic_type() {
    for src in [
        "inbox >> const value: Result<Option<int>, Error>;",
        "inbox >> val value: io.Result<core.Option<bytes.Buffer>>;",
        "source >> let part: Package::Buffer;",
    ] {
        let ops = parse(src);
        assert_eq!(ops.len(), 1, "{src}: {ops:?}");
    }
}

#[test]
fn incomplete_generic_annotation_and_trailing_tokens_fail_closed() {
    for src in [
        "inbox >> const x: ;",
        "inbox >> const x: Result<int;",
        "inbox >> const x: Result<int,>;",
        "inbox >> const x: Result<>;",
        "inbox >> const x: Result<Option<int>> extra;",
        "inbox >> const x: Thing bad;",
        "inbox >> const x: Thing<Other>>;",
        "inbox >> const x: Thing<Other,>;",
        "inbox >> const x: A::;",
        "inbox >> const x: A..B;",
        "inbox >> const x: A<B<C>;",
    ] {
        let diag = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(diag.code, "E_IO_SYNTAX", "{src}: {diag}");
    }
}

#[test]
fn incomplete_select_binding_never_silently_emits_guest_code() {
    for src in [
        "do select { when inbox >> const x: Result<Option<int>> extra -> {} }",
        "do select { when inbox >> const x: Result<> -> {} }",
    ] {
        let diagnostic = atto_compiler::compile(src).expect_err(src);
        assert_eq!(diagnostic.code, "E_IO_SYNTAX");
    }
}
