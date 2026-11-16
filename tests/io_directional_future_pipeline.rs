//! Syntax-only preflight for explicit nonblocking I/O future binding,
//! suspending lambda application, and terminal directional capture.
//! The browser guest stdio read ABI is not implemented; a valid parse MUST
//! still fail closed and produce no executable Wasm module.
use atto_compiler::compile;

fn program(statement: &str) -> String {
    format!("pub suspends fnc main(): int {{ {statement} return 42; }}")
}

#[test]
fn directional_future_await_read_flow_is_recognized_but_never_emitted_as_guest_io() {
    for statement in [
        "nb stdio.in >> val pending: Future<string> |> susp |f| -> await f >> val line: string;",
        "nb stdio.in >> val Future<string> pending |> suspends |f| -> await f >> val string line;",
        "nb stdio.in >> let pending: Future<Result<Option<string>,IoError>> |> susp |item| -> await item >> val outcome: Result<Option<string>,IoError>;",
        "nb stdio.in >> val pending: Future<Result<Option<string>,IoError>> |> suspends |p| -> await p >> let outcome: Result<Option<string>,IoError>;",
    ] {
        let diag = compile(&program(statement)).expect_err(
            "parser recognition must not falsely claim typed stdin guest execution",
        );
        assert_eq!(diag.code, "E_UNSUPPORTED", "{statement}: {diag}");
        assert!(
            diag.message.contains("read") || diag.message.contains("nonblocking"),
            "{statement}: expected explicit missing read-ABI diagnostic, got {diag}",
        );
    }
}

#[test]
fn directional_future_await_binder_and_type_errors_fail_closed() {
    for (statement, expected) in [
        ("nb stdio.in >> val pending: Future<int> |> susp |v| -> await wrong >> val value: int;", "E_PIPE_BIND"),
        ("nb stdio.in >> val pending: Future<int> |> susp |v| -> await v >> val value: string;", "E_TYPE"),
        ("nb stdio.in >> val pending: Future<Result<Option<string>,IoError>> |> susp |v| -> await v >> val value: string;", "E_TYPE"),
        ("nb stdio.in >> const pending: Future<int> |> susp |v| -> await v >> val value: int;", "E_CONST_EFFECT"),
        ("nb stdio.in >> val pending: Future<int> |> susp |v| -> await v >> const value: int;", "E_CONST_EFFECT"),
        ("nb stdio.in >> val pending: int |> susp |v| -> await v >> val value: int;", "E_IO_SYNTAX"),
    ] {
        let diag = compile(&program(statement)).expect_err("invalid read flow must fail");
        assert_eq!(diag.code, expected, "{statement}: {diag}");
    }
}

#[test]
fn await_is_not_a_function_and_future_type_is_not_a_reverse_pipe_value() {
    for statement in [
        "nb stdio.in >> val pending: Future<string> |> await pending >> val line: string;",
        "nb stdio.in >> val pending: Future<string> |> susp |f| -> await f <| Future<string> >> val line: string;",
        "nb stdio.in >> (val string line << |x| -> await x <| Future<string>);",
        "nb stdio.in >> Future<string> x |> await x >> val string line;",
    ] {
        compile(&program(statement)).expect_err(
            "type placeholders and bare await may not be runtime callable stages",
        );
    }
}
