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
fn stdout_shorthand_and_explicit_write_compile_to_same_guest_as_print() {
    let source = r#"pub fnc main(): int {
        val x: int = 42;
        stdio.out << "effectful output";
        writech stdio.out, "!";
        (stdio.out) << " bracketed ";
        stdio.out << x;
        return 7;
    }"#;
    let explicit = r#"pub fnc main(): int {
        val x: int = 42;
        stdio.print("effectful output");
        stdio.print("!");
        stdio.print(" bracketed ");
        stdio.print(x);
        return 7;
    }"#;
    let a = atto_compiler::compile(source).expect("standalone stdout shorthand executes");
    let b = atto_compiler::compile(explicit).expect("explicit stdout print executes");
    assert_eq!(&a[..8], b"\0asm\x01\0\0\0");
    assert_eq!(a, b, "the typed checked guest I/O lowering must be identical");
}

#[test]
fn unsupported_channel_stream_and_select_io_fail_closed() {
    for src in [
        "pub fnc main(): void { readch inbox; }",
        "pub fnc main(): void { writech outgoing, \"msg\"; }",
        "pub fnc main(): void { output_stream << \"msg\"; }",
        "pub fnc main(): void { stdio.in >> val line; }",
        "pub fnc main(): void { stdio.in << \"bad direction\"; }",
        "pub fnc main(): void { do select { when inbox >> val msg -> {} } }",
    ] {
        let err = atto_compiler::compile(src).expect_err("unsupported guest I/O must never emit Wasm");
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}: {err}");
    }
}

#[test]
fn stdout_shorthand_has_typechecker_and_lexical_guards() {
    let invalid_type = r#"pub fnc main(): void { stdio.out << Option.Some(42); }"#;
    assert!(atto_compiler::compile(invalid_type).is_err(),
        "unsupported payloads never bypass typed stdio.print");
    let spaced = "pub fnc main(): void { stdio.out < < 42; }";
    assert!(atto_compiler::compile(spaced).is_err(),
        "whitespace-separated comparisons must not become I/O");
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
    (input_stream) >> val chunk;
    do select {
        when (inbox) >> val msg -> { (stdio.out) << msg; }
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
    assert!(parse("do select { when inbox >> val x: Result<Option<int>> -> {} }").len() == 1);
}


#[test]
fn select_guards_require_immediately_enclosing_select_block() {
    for src in [
        "when inbox >> val value -> {}",
        "pub fnc main(): void { when inbox >> val value -> {} }",
        "pub fnc main(): void { if true then { when inbox >> val value -> {} } fi }",
        "do select { if true then { when inbox >> val value -> {} } fi }",
        "do { when output << \"data\" -> {} }",
    ] {
        let diag = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(diag.code, "E_IO_SYNTAX", "{src}: {diag}");
        assert!(diag.message.contains("directly inside select"), "{src}: {diag}");
    }
}

#[test]
fn select_guards_are_still_recognized_after_nested_arm_bodies() {
    let sample = r#"do select {
      when inbox >> val first -> { if true then { stdio.out << "ok"; } fi }
      when outgoing << "second" -> { stdio.out << "done"; }
    }"#;
    let ops = parse(sample);
    assert_eq!(ops.len(), 4, "{ops:?}");
    assert_eq!(ops[0].context, Context::SelectArm);
    assert_eq!(ops[1].context, Context::Standalone);
    assert_eq!(ops[2].context, Context::SelectArm);
    assert_eq!(ops[3].context, Context::Standalone);
}

#[test]
fn read_binders_cannot_shadow_reserved_language_keywords() {
    for binder in ["if", "for", "return", "await", "fnc", "class", "module", "select", "when"] {
        for source in [
            format!("inbox >> val {binder};"),
            format!("val {binder} << inbox;"),
            format!("do select {{ when inbox >> let {binder} -> {{}} }}"),
            format!("do select {{ when let {binder} << inbox -> {{}} }}"),
        ] {
            let err = inspect(&lex(&source).unwrap()).expect_err(&source);
            assert_eq!(err.code, "E_IO_SYNTAX", "{source}: {err}");
        }
    }
}

#[test]
fn typed_select_generic_guard_context_is_preserved() {
    for src in [
        "do select<int> { when inbox >> val x -> {} }",
        "do select<Result<Option<int>, Error>> { when inbox >> let x -> {} }",
        "nb select<Result<Option<int>, Error>> { when outgoing << value -> {} }",
    ] {
        let ops = parse(src);
        assert_eq!(ops.len(), 1, "{src}: {ops:?}");
        assert_eq!(ops[0].context, Context::SelectArm, "{src}");
    }
}

#[test]
fn typed_select_is_not_inherited_by_nested_non_select_blocks() {
    for src in [
        "do select<int> { block { when inbox >> val x -> {} } }",
        "do select<Result<Option<int>, Error>> { if true then { when inbox >> val x -> {} } fi }",
    ] {
        let err = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(err.code, "E_IO_SYNTAX", "{src}: {err}");
    }
}

#[test]
fn lexically_shadowed_stdio_cannot_bypass_typed_receiver_resolution() {
    for src in [
        r#"pub fnc main(): int {
              val stdio: int = 1;
              stdio.out << "not a builtin handle";
              return 7;
            }"#,
        r#"pub fnc main(): int {
              val stdio: int = 1;
              (stdio.out) << "not a builtin handle";
              return 7;
            }"#,
        r#"pub fnc main(): int {
              val stdio: int = 1;
              writech stdio.out, "not a builtin handle";
              return 7;
            }"#,
        r#"pub fnc main(): int {
              val stdio: int = 1;
              stdio.print("not a builtin handle");
              return 7;
            }"#,
    ] {
        let err = atto_compiler::compile(src).expect_err(src);
        assert_eq!(err.code, "E_IO_SHADOW", "{src}: {err}");
    }
}


#[test]
fn nb_and_try_preserve_mode_and_future_binding_without_conflating_plain_io() {
    use atto_compiler::io_shorthand::Mode;
    let src = r#"
nb inbox >> val pending: Future<Result<Option<string>, IoError>>;
nb outgoing << "hi";
try inbox >> let immediate;
try outgoing << "hi";
inbox >> val completed;
outgoing << "synchronous";
do nb select {
    when inbox >> val selected -> {}
    when outgoing << "ready" -> {}
}
"#;
    let parsed = parse(src);
    assert_eq!(parsed.len(), 8, "{parsed:?}");
    assert_eq!(parsed[0].mode, Mode::Nonblocking);
    assert_eq!(parsed[0].direction, Direction::Read);
    assert_eq!(parsed[0].binder.as_deref(), Some("pending"));
    assert_eq!(parsed[1].mode, Mode::Nonblocking);
    assert_eq!(parsed[1].direction, Direction::Write);
    assert_eq!(parsed[2].mode, Mode::TryImmediate);
    assert_eq!(parsed[2].binder.as_deref(), Some("immediate"));
    assert_eq!(parsed[3].mode, Mode::TryImmediate);
    for i in [4, 5, 6, 7] {
        assert_eq!(parsed[i].mode, Mode::Direct);
    }
    assert_eq!(parsed[6].context, Context::SelectArm);
    assert_eq!(parsed[7].context, Context::SelectArm);
}

#[test]
fn nb_try_explicit_channel_operations_and_select_guard_restrictions() {
    for src in [
        "nb readch foo >> Future<string> foo;",
        "nb readch foo >> val pending;",
        "when nb inbox >> val x -> {}",
        "do select { when nb inbox >> val x -> {} }",
        "do select { when try inbox >> val x -> {} }",
        "do nb select { when nb outgoing << 1 -> {} }",
        "nb val pending << inbox;",
        "try val item << inbox;",
        "nb inbox >> const pending;",
        "try inbox >> const immediate;",
    ] {
        let d = inspect(&lex(src).unwrap()).expect_err(src);
        assert!(matches!(d.code, "E_IO_SYNTAX" | "E_CONST_EFFECT"), "{src}: {d}");
    }
}

#[test]
fn unsupported_io_and_effectless_nb_reads_cannot_emit_guest_wasm() {
    for (src, code) in [
        ("pub fnc main(): void { nb stdio.out << \"hi\"; }", "E_UNSUPPORTED"),
        ("pub fnc main(): void { try stdio.out << \"hi\"; }", "E_UNSUPPORTED"),
        ("pub fnc main(): void { nb stdio.in >> val pending; }", "E_UNSUPPORTED"),
        ("pub fnc main(): void { nb writech foo, \"hi\"; }", "E_UNSUPPORTED"),
        // Source-level channel reads have real lowering only inside an
        // explicitly suspending function. A missing effect is E_EFFECT,
        // never a silently synchronous read or an emitted Wasm artifact.
        ("pub fnc main(): void { val pending = nb readch foo; }", "E_EFFECT"),
    ] {
        let err = atto_compiler::compile(src).expect_err(src);
        assert_eq!(err.code, code, "{src}: {err}");
    }
}

#[test]
fn deeply_generic_future_annotations_are_not_direction_operators() {
    use atto_compiler::io_shorthand::Mode;
    let parsed = parse("nb inbox >> val p: Future<Result<Option<Result<string, IoError>>, Error>>;");
    assert_eq!(parsed.len(), 1, "{parsed:?}");
    assert_eq!(parsed[0].mode, Mode::Nonblocking);
    assert_eq!(parsed[0].binder.as_deref(), Some("p"));
}


#[test]
fn spaced_optional_equals_in_plain_read_and_select_binders_is_a_noop() {
    use atto_compiler::io_shorthand::Mode;
    let source = r#"
stdio.in >> = val line: Result<Option<string>, IoError>;
nb inbox >> = val pending: Future<Result<Option<string>, IoError>>;
try inbox >> = let immediate;
do select {
    when inbox >> = val selected -> {}
}
"#;
    let ops = parse(source);
    assert_eq!(ops.len(), 4, "{ops:?}");
    assert_eq!(ops[0].source_or_sink, "stdio.in");
    assert_eq!(ops[0].binder.as_deref(), Some("line"));
    assert_eq!(ops[1].mode, Mode::Nonblocking);
    assert_eq!(ops[1].binder.as_deref(), Some("pending"));
    assert_eq!(ops[2].mode, Mode::TryImmediate);
    assert_eq!(ops[2].binder.as_deref(), Some("immediate"));
    assert_eq!(ops[3].context, Context::SelectArm);
    assert_eq!(ops[3].binder.as_deref(), Some("selected"));

    for src in [
        "pub fnc main(): void { stdio.in >> = val line; }",
        "pub fnc main(): void { nb stdio.in >> = val pending; }",
    ] {
        let diag=atto_compiler::compile(src).expect_err(
            "recognized read shorthand must fail closed without guest I/O");
        assert_eq!(diag.code,"E_UNSUPPORTED","{src}: {diag}");
    }
}

#[test]
fn spaced_optional_equals_does_not_accept_shift_assignment_or_repeated_equals() {
    for src in [
        "inbox >>= val line;",
        "inbox >>== val line;",
        "inbox >> == val line;",
        "inbox >> = = val line;",
        "inbox >> = ;",
        "inbox >> = existing_line;",
        "inbox >> = const line;",
    ] {
        let diag=inspect(&lex(src).unwrap()).expect_err(src);
        assert!(
            matches!(diag.code,"E_IO_SYNTAX" | "E_CONST_EFFECT"),
            "{src}: {diag}"
        );
    }
    assert!(parse("scalar > > = val x;").is_empty());
    assert!(parse("pub fnc main(): int { return 3 >> 1; }").is_empty());
}
