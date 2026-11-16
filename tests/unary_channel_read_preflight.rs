//! Unary channel/stream receive expression: lexical-preflight contract only.
//! No guest I/O execution can be inferred from these tests.
use atto_compiler::{compile, io_shorthand, lexer};

fn reads(source: &str) -> Result<Vec<io_shorthand::UnaryReadCandidate>, atto_compiler::Diagnostic> {
    let tokens = lexer::lex(source)?;
    io_shorthand::inspect_unary_reads(&tokens)
}

#[test]
fn unary_receive_is_an_expression_in_assignment_call_and_return_positions() {
    let src = "val a = << c; total += (<< c); process(<< source.inbox); return << (c);";
    let found = reads(src).expect("syntax preflight");
    let names: Vec<_> = found.iter().map(|x| x.source.as_str()).collect();
    assert_eq!(names, ["c", "c", "source.inbox", "c"]);
}

#[test]
fn two_separate_reads_in_the_same_expression_are_not_a_directional_chain() {
    let found = reads("val sum = (<< c) + (<< c);").expect("two reads");
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|r| r.source == "c"));
    assert_ne!(found[0].pos.col, found[1].pos.col);
}

#[test]
fn unary_receive_never_steals_writes_reverse_binders_or_numeric_shifts() {
    for source in [
        "stdio.out << 42;",
        "val x << source;",
        "let result: Result<Option<int>, IoError> << inbox;",
        "val shifted = 1 << 3;",
        "val computed = foo << value;",
        "val nested: Map<int, Option<int>> = value;",
        "val x = a < <b;",
        "out >> val msg;",
        "readable pipe writable;",
    ] {
        assert!(reads(source).unwrap().is_empty(), "false unary read: {source}");
    }
}

#[test]
fn incomplete_discarded_and_const_runtime_reads_have_stable_diagnostics() {
    for (source, code) in [
        ("val x = << ;", "E_IO_SYNTAX"),
        ("val x = << ();", "E_IO_SYNTAX"),
        ("val x = << (c;", "E_IO_SYNTAX"),
        ("<< c;", "E_IO_DISCARD"),
        ("const x = << c;", "E_CONST_EFFECT"),
    ] {
        let error = reads(source).expect_err(source);
        assert_eq!(error.code, code, "{source}: {error}");
    }
}

#[test]
fn currently_unimplemented_guest_channel_read_must_never_emit_wasm() {
    for expression in [
        "val x = << c;",
        "total += (<< c);",
        "val sum = (<< c) + (<< c);",
        "process(<< c);",
        "return << c;",
        "val result = trap << c;",
    ] {
        let src = format!("pub suspends fnc main(): int using io, suspend {{ {expression} return 42; }}");
        let err = compile(&src).expect_err("typed read import not implemented");
        assert_eq!(err.code, "E_UNSUPPORTED", "{expression}: {err}");
        assert!(err.message.contains("typed guest"), "{expression}: {err}");
    }
}

#[test]
fn type_first_declaration_reads_are_recognized_without_becoming_binary_writes() {
    for (source, binding, channel) in [
        ("val int a << c;", "a", "c"),
        ("let Result<Option<int>, IoError> result << inbox;", "result", "inbox"),
        ("val a: Result<Option<int>, IoError> << inbox;", "a", "inbox"),
        ("val Channel<int> received << channel_source;", "received", "channel_source"),
    ] {
        let tokens = lexer::lex(source).unwrap();
        let directional = io_shorthand::inspect(&tokens).expect(source);
        assert_eq!(directional.len(), 1, "{source}");
        let read = &directional[0];
        assert_eq!(read.direction, io_shorthand::Direction::Read, "{source}");
        assert!(read.read_binder_on_left, "{source}");
        assert_eq!(read.binder.as_deref(), Some(binding), "{source}");
        assert_eq!(read.source_or_sink, channel, "{source}");
        assert!(reads(source).unwrap().is_empty(), "binary declaration stolen by unary parse: {source}");
    }
}

#[test]
fn type_first_bindings_preserve_const_and_complete_type_errors() {
    for (source, code) in [
        ("const int a << c;", "E_CONST_EFFECT"),
        ("val Result<Option<int>, IoError a << inbox;", "E_IO_SYNTAX"),
        ("val int a unexpected << c;", "E_IO_SYNTAX"),
    ] {
        let tokens = lexer::lex(source).unwrap();
        let error = io_shorthand::inspect(&tokens).expect_err(source);
        assert_eq!(error.code, code, "{source}: {error}");
    }
}

#[test]
fn prefix_and_declaration_first_variants_do_not_emit_guest_read_wasm() {
    for body in [
        "val int a = << c; return 0;",
        "val int a = (<< c); return 0;",
        "val int a << c; return 0;",
        "let total = 0; total += << c; return total;",
        "let total = 0; total += (<< c); return total;",
    ] {
        let source = format!("pub suspends fnc main(): int using io, suspend {{ {body} }}");
        let error = compile(&source).expect_err("guest channel reads are not yet lowered");
        assert_eq!(error.code, "E_UNSUPPORTED", "{body}: {error}");
    }
}
