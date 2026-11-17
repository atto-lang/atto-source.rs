//! Conformance preflight for typed directional Future/result capture.
//! Both requested forms MUST parse, but stdin guest ABI remains unsupported.
//! Parsing an await or propagating Result.Err must never synthesize fake Wasm.
use atto_compiler::compile;

fn program(statement: &str) -> String {
    format!("pub suspends fnc main(): int {{ {statement} return 42; }}")
}

#[test]
fn named_future_forward_pipe_and_anonymous_reverse_pipe_are_recognized() {
    for statement in [
        "nb stdio.in >> (val String pending_line << |x| -> await x <| Future<string>);",
        "nb stdio.in >> Future<string> x |> |f| -> await f >> val String pending_line;",
        // Compatibility with existing contextual separator and typed continuations.
        "nb stdio.in >> Future<string> x |> susp |f| -> await f >> val String pending_line;",
        "nb stdio.in >> (val Option<string> line << |f| -> (await f)? <| Future<Result<Option<string>,IoError>>);",
        "nb stdio.in >> Future<Result<Option<string>,IoError>> future |> |f| -> (await f)? >> val Option<string> line;",
        "nb stdio.in >> (val Option<string> line << susp |f| -> (await f)? <| Future<Result<Option<string>,IoError>>);",
        "nb stdio.in >> val future: Future<Result<Option<string>,IoError>> |> suspends |f| -> (await f)? >> val Option<string> line;",
        "nb stdio.in >> Future<Result<string,IoError>> future |> |f| -> (await f)? >> val string line;",
    ] {
        let diag = compile(&program(statement))
            .expect_err("stdin guest read ABI must not emit executable Wasm");
        assert_eq!(diag.code, "E_UNSUPPORTED", "{statement}: {diag}");
        assert!(diag.message.contains("guest ABI"), "{statement}: {diag}");
    }
}

#[test]
fn named_future_capture_enforces_identity_and_checked_propagation() {
    for (statement, expected) in [
        ("nb stdio.in >> Future<string> x |> await x >> val String line;", "E_SYNTAX"),
        ("nb stdio.in >> Future<string> x |> (await x)? >> val string line;", "E_SYNTAX"),
        ("nb stdio.in >> Future<string> x | await x >> val String line;", "E_SYNTAX"),
        ("nb stdio.in >> Future<string> x |> |f| -> await y >> val String line;", "E_PIPE_BIND"),
        ("nb stdio.in >> Future<string> x |> |f| -> (await y)? >> val string line;", "E_PIPE_BIND"),
        ("nb stdio.in >> Future<string> x |> |f| -> (await f)? >> val string line;", "E_TYPE"),
        ("nb stdio.in >> Future<Result<Option<string>,IoError>> x |> |f| -> await f >> val Option<string> line;", "E_TYPE"),
        ("nb stdio.in >> Future<Result<Option<string>,IoError>> x |> |f| -> (await f)? >> val string line;", "E_TYPE"),
        ("nb stdio.in >> Future<Result<Option<string>,IoError>> x |> |f| -> await f? >> val Option<string> line;", "E_IO_SYNTAX"),
        ("nb stdio.in >> Future<Result<string,IoError>> x |> |f| -> await f? >> val string line;", "E_IO_SYNTAX"),
        ("nb stdio.in >> Future<string> x |> |f| -> await f >> const string line;", "E_CONST_EFFECT"),
    ] {
        let diag = compile(&program(statement)).expect_err("bad capture must fail closed");
        assert_eq!(diag.code, expected, "{statement}: {diag}");
    }
}

#[test]
fn reversed_lambda_pipe_checks_binding_and_lexical_delimiters() {
    for (statement, expected) in [
        ("nb stdio.in >> (val string line << |x| -> await other <| Future<string>);", "E_PIPE_BIND"),
        ("nb stdio.in >> (val string line << |x| -> (await x)? <| Future<string>);", "E_TYPE"),
        ("nb stdio.in >> (val string line << |x| -> await x < | Future<string>);", "E_SYNTAX"),
        ("nb stdio.in >> (val string line << |x| -> await x <| Future<int>);", "E_TYPE"),
        ("nb stdio.in >> (const string line << |x| -> await x <| Future<string>);", "E_CONST_EFFECT"),
    ] {
        let diag = compile(&program(statement)).expect_err("malformed reverse capture must reject");
        assert_eq!(diag.code, expected, "{statement}: {diag}");
    }
}

#[test]
fn bare_await_stage_remains_forbidden_outside_contextual_named_future_capture() {
    let src = "pub suspends fnc main(): int { val x=task {return 42;}; return x |> await; }";
    let diag = compile(src).expect_err("await is not a generic callable stage");
    assert_eq!(diag.code, "E_UNSUPPORTED");
}


#[test]
fn optional_equals_is_accepted_independently_at_both_nb_directional_bindings() {
    // The two spellings remain parser-only: a syntactically valid read must
    // reach the explicit unsupported guest ABI gate, never emit fake Wasm.
    for (initial, terminal) in [("", ""), ("= ", ""), ("", "= "), ("= ", "= ")] {
        let operation = format!(
            "nb stdio.in >> {initial}Future<Result<Option<string>,IoError>> x \
             |> |f| -> (await f)? >> {terminal}val Option<string> pending_line;"
        );
        let diag = compile(&program(&operation))
            .expect_err("guest stdin ABI is not present");
        assert_eq!(diag.code, "E_UNSUPPORTED", "{operation}: {diag}");
        assert!(diag.message.contains("guest ABI"), "{operation}: {diag}");
    }
}

#[test]
fn optional_equals_also_accepts_explicit_val_future_binder_and_reverse_capture() {
    for operation in [
        "nb stdio.in >> = val pending: Future<Result<Option<string>,IoError>> |> susp |f| -> (await f)? >> = val Option<string> line;",
        "nb stdio.in >> = (val Option<string> line << |f| -> (await f)? <| Future<Result<Option<string>,IoError>>);",
    ] {
        let diag = compile(&program(operation))
            .expect_err("typed stdin read is not guest executable");
        assert_eq!(diag.code, "E_UNSUPPORTED", "{operation}: {diag}");
        assert!(diag.message.contains("guest ABI"), "{operation}: {diag}");
    }
}

#[test]
fn direction_equals_is_one_optional_spaced_marker_not_an_assignment_operator() {
    for operation in [
        "nb stdio.in >>= Future<string> x |> |f| -> await f >> val string line;",
        "nb stdio.in >>== Future<string> x |> |f| -> await f >> val string line;",
        "nb stdio.in >> == Future<string> x |> |f| -> await f >> val string line;",
        "nb stdio.in >> = = Future<string> x |> |f| -> await f >> val string line;",
        "nb stdio.in >> = Future<string> x |> |f| -> await f >>= val string line;",
        "nb stdio.in >> = Future<string> x |> |f| -> await f >> == val string line;",
        "nb stdio.in >> = Future<string> x |> |f| -> await f >> = = val string line;",
        "nb stdio.in >> = Future<string> x |> |f| -> await f >> = pending_line;",
        "nb stdio.in >> = Future<string> x |> |f| -> await f >> = const string line;",
    ] {
        let diag = compile(&program(operation))
            .expect_err("no arbitrary assignment, shift, or repeated direction marker");
        assert_ne!(diag.code, "E_UNSUPPORTED",
            "invalid grammar must not reach missing-ABI gate: {operation}: {diag}");
    }
}

#[test]
fn contextual_equals_does_not_erase_result_or_option_payload() {
    for operation in [
        "nb stdio.in >> = Future<Result<Option<string>,IoError>> x |> |f| -> await f >> = val Option<string> line;",
        "nb stdio.in >> = Future<Result<Option<string>,IoError>> x |> |f| -> (await f)? >> = val string line;",
        "nb stdio.in >> = Future<string> x |> |f| -> (await f)? >> = val string line;",
    ] {
        let diag = compile(&program(operation)).expect_err("lossy binding must be rejected");
        assert_eq!(diag.code, "E_TYPE", "{operation}: {diag}");
    }
}


#[test]
fn marked_nb_continuation_requires_a_fresh_typed_target_even_without_guest_abi() {
    // Bypass the broad source I/O preflight to test the parser's own
    // fail-closed invariant. A malformed marked capture cannot masquerade
    // as a valid-but-unimplemented stdin Future.
    use atto_compiler::{lexer::lex, parser::Parser};
    for statement in [
        "nb stdio.in >> = existing_variable;",
        "nb stdio.in >> = ;",
        "nb stdio.in >> = = Future<string> x |> |f| -> await f >> val string line;",
    ] {
        let source = program(statement);
        let d = Parser::new(lex(&source).unwrap())
            .parse_program().expect_err("invalid marked binder must be rejected");
        assert_eq!(d.code, "E_IO_SYNTAX", "{statement}: {d}");
    }
}
