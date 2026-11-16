use atto_compiler::io_shorthand::{inspect, Context, Direction};
use atto_compiler::lexer::lex;

fn parse(input: &str) -> Vec<atto_compiler::io_shorthand::IoShorthand> {
    inspect(&lex(input).unwrap()).unwrap()
}

#[test]
fn recognizes_standalone_channel_and_stream_io() {
    let sample = r#"
stdio.out << "Hello";
stdio.in >> val line;
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
    when inbox >> val message -> {
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
        "inbox >> val ;",
        "outgoing << ;",
        "outgoing << value << other;",
        "do select { when inbox >> message -> {} }",
        "do select { when outgoing << -> {} }",
        "do select { when incoming >> val -> {} }",
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
        "inbox >> val value: Result<Option<int>, Error>;",
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
        "inbox >> val x: ;",
        "inbox >> val x: Result<int;",
        "inbox >> val x: Result<int,>;",
        "inbox >> val x: Result<>;",
        "inbox >> val x: Result<Option<int>> extra;",
        "inbox >> val x: Thing bad;",
        "inbox >> val x: Thing<Other>>;",
        "inbox >> val x: Thing<Other,>;",
        "inbox >> val x: A::;",
        "inbox >> val x: A..B;",
        "inbox >> val x: A<B<C>;",
    ] {
        let diag = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(diag.code, "E_IO_SYNTAX", "{src}: {diag}");
    }
}

#[test]
fn incomplete_select_binding_never_silently_emits_guest_code() {
    for src in [
        "do select { when inbox >> val x: Result<Option<int>> extra -> {} }",
        "do select { when inbox >> val x: Result<> -> {} }",
    ] {
        let diagnostic = atto_compiler::compile(src).expect_err(src);
        assert_eq!(diagnostic.code, "E_IO_SYNTAX");
    }
}

#[test]
fn reversed_bindings_read_from_rhs_without_becoming_writes() {
    let src = r#"
val line << stdio.in;
let chunk: Buffer << file.input;
stdio.in >> val second;
do select {
    when val received << inbox -> {
        stdio.out << received;
    }
    when let update: int << changes -> {
        output << update;
    }
}
"#;
    let ops = parse(src);
    assert_eq!(ops.len(), 7, "{ops:?}");
    for i in [0, 1, 3, 5] {
        assert_eq!(ops[i].direction, Direction::Read, "{ops:?}");
        assert!(ops[i].read_binder_on_left);
    }
    assert_eq!(ops[0].source_or_sink, "stdio.in");
    assert_eq!(ops[0].binder.as_deref(), Some("line"));
    assert_eq!(ops[1].source_or_sink, "file.input");
    assert_eq!(ops[1].binder.as_deref(), Some("chunk"));
    assert_eq!(ops[2].direction, Direction::Read);
    assert!(!ops[2].read_binder_on_left);
    assert_eq!(ops[3].context, Context::SelectArm);
    assert_eq!(ops[4].direction, Direction::Write);
    assert_eq!(ops[5].context, Context::SelectArm);
    assert_eq!(ops[6].direction, Direction::Write);
}

#[test]
fn runtime_const_reads_are_rejected_in_both_directions() {
    for src in [
        "stdio.in >> const line;",
        "const line << stdio.in;",
        "do select { when inbox >> const message -> {} }",
        "do select { when const message << inbox -> {} }",
        "const line: string << stdio.in;",
    ] {
        let diag = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(diag.code, "E_CONST_EFFECT", "{src}: {diag}");
        assert!(diag.message.contains("compile-time"), "{src}: {diag}");
    }
}

#[test]
fn malformed_reversed_read_binding_fails_closed() {
    for src in [
        "val << stdio.in;",
        "val line << ;",
        "val line << input() ;",
        "val line << input << other;",
        "do select { when val << inbox -> {} }",
        "do select { when let line << -> {} }",
        "val line: Result<int,> << inbox;",
        "val line: Option<int>> << inbox;",
    ] {
        let diag = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(diag.code, "E_IO_SYNTAX", "{src}: {diag}");
    }
}

#[test]
fn nested_generic_closers_without_io_do_not_trigger_preflight() {
    for src in [
        "pub fnc main(): void { val x: A<B<C>> = new Something(); }",
        "pub fnc main(): int { return f<A<B<C>>>(1); }",
    ] {
        assert!(parse(src).is_empty(), "{src}");
    }
}

#[test]
fn parentheses_around_qualified_io_receiver_are_supported() {
    let source = r#"
    (stdio.out) << "greeting";
    (input_stream) >> const chunk;
    do select {
        when (inbox) >> const msg -> { (stdio.out) << msg; }
        when (outgoing) << "ready" -> {}
    }
    "#;
    let ops = parse(source);
    assert_eq!(ops.len(), 5, "{ops:?}");
    assert_eq!(ops[0].source_or_sink, "stdio.out");
    assert_eq!(ops[1].source_or_sink, "input_stream");
    assert_eq!(ops[2].context, Context::SelectArm);
    assert_eq!(ops[3].source_or_sink, "stdio.out");
    assert_eq!(ops[4].direction, Direction::Write);
}

#[test]
fn standalone_direction_does_not_reinterpret_expression_context() {
    assert!(parse("const x: int = 1 << 3;").is_empty());
    assert!(parse("return n << 1;").is_empty());
    assert!(parse("do select { when inbox >> const x: Result<Option<int>> -> {} }").len() == 1);
}
