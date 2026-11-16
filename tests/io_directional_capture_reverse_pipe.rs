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
        "nb stdio.in >> Future<string> x |> await x >> val String pending_line;",
        // Compatibility with existing contextual separator and typed continuations.
        "nb stdio.in >> Future<string> x | await x >> val String pending_line;",
        "nb stdio.in >> (val Option<string> line << |f| -> (await f)? <| Future<Result<Option<string>,IoError>>);",
        "nb stdio.in >> Future<Result<Option<string>,IoError>> future |> (await future)? >> val Option<string> line;",
        "nb stdio.in >> (val Option<string> line << susp |f| -> (await f)? <| Future<Result<Option<string>,IoError>>);",
        "nb stdio.in >> val future: Future<Result<Option<string>,IoError>> |> suspends |f| -> (await f)? >> val Option<string> line;",
        "nb stdio.in >> Future<Result<string,IoError>> future |> (await future)? >> val string line;",
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
        ("nb stdio.in >> Future<string> x |> await y >> val String line;", "E_PIPE_BIND"),
        ("nb stdio.in >> Future<string> x |> (await y)? >> val string line;", "E_PIPE_BIND"),
        ("nb stdio.in >> Future<string> x |> (await x)? >> val string line;", "E_TYPE"),
        ("nb stdio.in >> Future<Result<Option<string>,IoError>> x |> await x >> val Option<string> line;", "E_TYPE"),
        ("nb stdio.in >> Future<Result<Option<string>,IoError>> x |> (await x)? >> val string line;", "E_TYPE"),
        ("nb stdio.in >> Future<Result<Option<string>,IoError>> x |> await x? >> val Option<string> line;", "E_IO_SYNTAX"),
        ("nb stdio.in >> Future<Result<string,IoError>> x |> await x? >> val string line;", "E_IO_SYNTAX"),
        ("nb stdio.in >> Future<string> x |> await x >> const string line;", "E_CONST_EFFECT"),
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
