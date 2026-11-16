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
