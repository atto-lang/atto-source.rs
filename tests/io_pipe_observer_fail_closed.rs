//! Fail-closed regression for candidate pipe observers and io.pipe.
//!
//! These are not runtime tests: this compiler does NOT implement a
//! PipeHandle observer ABI or io.pipe transport. Until it does,
//! accepted-looking code must never silently produce a Wasm guest.
use atto_compiler::compile;

const PRELUDE: &str = r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void { emit(mail.value); }
end
"#;

fn compile_main(body: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile(&format!("{PRELUDE}\npub fnc main(): int {{\n{body}\nreturn 0;\n}}"))
}

#[test]
fn io_pipe_must_not_be_emitted_as_an_unchecked_void_or_bool_operation() {
    for body in [
        "val graph = io.pipe(Stream.port<int>(1).stream(), Stream.port<int>(1).sink());",
        "val graph = io.pipe(Stream.port<int>(1).sink(), Stream.port<int>(1).stream());",
        "val graph = io.pipe(spawn Producer(), Stream.port<int>(1).sink());",
        "io.pipe(spawn Producer(), Stream.port<int>(1).sink());",
    ] {
        assert!(compile_main(body).is_err(),
            "unimplemented io.pipe must not emit a guest: {body}");
    }
}

#[test]
fn trailing_observer_must_not_be_mistaken_for_an_additional_data_sink() {
    for body in [
        "val g = spawn Producer() pipe Stream.port<int>(1).sink() pipe observer(#error, |e| -> { });",
        "val g = Stream.port<int>(1).sink() rpipe spawn Producer() rpipe observer(#error, |e| -> { });",
        "val g = spawn Producer() pipe observer(|| -> { }) pipe Stream.port<int>(1).sink();",
        "val g = spawn Producer() pipe Stream.port<int>(1).sink() pipe observer(|| -> { });",
    ] {
        assert!(compile_main(body).is_err(),
            "observer syntax without a runtime and typed observer role must fail closed: {body}");
    }
}

#[test]
fn registering_a_callback_does_not_make_unimplemented_pipe_completion_safe() {
    for body in [
        "val g = spawn Producer() pipe Stream.port<int>(2).sink(); g.on(#error, |e| -> { });",
        "val g = Stream.port<int>(2).sink() rpipe spawn Producer(); g.on(#complete, |s| -> { });",
        "val g = io.pipe(Stream.port<int>(2).stream(), Stream.port<int>(2).sink()); g.on(#error, |e| -> { });",
    ] {
        assert!(compile_main(body).is_err(),
            "on(#event) must not hide unsupported graph completion: {body}");
    }
}

#[test]
fn existing_non_pipe_guest_compilation_remains_unchanged() {
    let binary=compile("pub fnc main(): int { return 41; }")
        .expect("existing scalar guest still compiles");
    assert!(binary.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn actor_local_io_cannot_implicitly_gain_process_stdio_pipe_permission() {
    let guest = r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void {
        val unexpected = io.pipe(io.in, io.out);
        emit(mail.value);
    }
end
pub fnc main(): int { return 0; }
"#;
    assert!(compile(guest).is_err(),
        "implicit actor io must not become a general process stdio pipe");
}


#[test]
fn unresolved_io_pipe_namespace_has_a_precise_fail_closed_diagnostic() {
    // No io namespace/bound local exists here. This is NOT a general
    // instance method named pipe and must not silently emit a host pump.
    let source = "pub fnc main(): int { val g = io.pipe(1, 2); return 0; }";
    let diagnostic = compile(source).expect_err("missing owned io.pipe ABI");
    assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{diagnostic}");
    assert!(diagnostic.message.contains("PipeHandle"), "{diagnostic}");
}


#[test]
fn io_pipe_parses_as_a_qualified_call_not_as_infix_pipe() {
    use atto_compiler::{lexer, parser::Parser};
    use atto_compiler::ast::{ExprKind, Stmt};
    let source = "pub fnc main(): int { val g = io.pipe(1, 2); return 0; }";
    let ast = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let function = ast.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &function.body[0] else { panic!("binding") };
    let ExprKind::MethodCall(receiver, member, args) = &init.kind else {
        panic!("io.pipe should be a method-call AST, not a pipeline AST");
    };
    assert!(matches!(&receiver.kind, ExprKind::Var(name) if name == "io"));
    assert_eq!(member, "pipe");
    assert_eq!(args.len(), 2);
    // No guest transport has been implemented, even though grammar is known.
    assert_eq!(compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn reserved_pipe_does_not_turn_into_a_general_member_name() {
    let source = "pub fnc main(): int { val g = other.pipe(1, 2); return 0; }";
    assert!(compile(source).is_err());
}
