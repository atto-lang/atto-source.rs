//! Compiler-facing checks for Atto VALUE pipeline effects versus continuous
//! GRAPH setup. Native/Wasm guest *execution* must be measured separately.
//! The companion spec Node tests are a host-only reference model.
use atto_compiler::compile;

#[test]
fn forward_pipeline_throw_reaches_caller_trap_not_downstream_call() {
    // Fixture runner must ALSO execute this guest and assert exit=9 and no
    // "BUG" stdout; Wasm header alone is not execution evidence.
    let src = include_str!("fixtures/pass/250-forward-pipeline-throw-trap-none.atto");
    let guest = compile(src).expect("ordinary throw inside a VALUE stage must compile");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(guest, compile(src).unwrap(), "guest generation is deterministic");
}

#[test]
fn forward_pipeline_result_err_remains_typed_data_inside_trap() {
    // Fixture runtime expectation: stdout="negative identifier", exit=7.
    // Incorrect implicit Err->throw conversion would instead reach None (99).
    let src = include_str!("fixtures/pass/251-forward-pipeline-result-err-is-data.atto");
    let guest = compile(src).expect("Err must remain Result DATA through |> stages");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(guest, compile(src).unwrap());
}

const ACTOR: &str = r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void { emit(mail.value); }
end
pub fnc main(): int {
    %BODY%
    return 0;
}
"#;

#[test]
fn enclosing_trap_never_overrides_a_static_graph_type_failure() {
    for body in [
        "val x = trap (spawn Producer() pipe Stream.port<bool>(2).sink());",
        "val x = trap (Stream.port<bool>(2).sink() rpipe spawn Producer());",
    ] {
        let diag = compile(&ACTOR.replace("%BODY%",body))
            .expect_err("trap cannot change a wrong port type into Option.None");
        assert_eq!(diag.code, "E_TYPE", "{body}: {diag}");
    }
}

#[test]
fn graph_admission_under_trap_has_no_synthetic_guest_or_pipe_handle() {
    for body in [
        "val x = trap (spawn Producer() pipe Stream.port<int>(2).sink());",
        "val x = trap (Stream.port<int>(2).sink() rpipe spawn Producer());",
    ] {
        let diag = compile(&ACTOR.replace("%BODY%",body))
            .expect_err("owned transport not implemented: no fake success under trap");
        assert_eq!(diag.code, "E_UNSUPPORTED", "{body}: {diag}");
    }
}

#[test]
fn mixed_graph_directions_are_an_error_even_under_trap() {
    let body="val x = trap (spawn Producer() pipe Stream.port<int>(2).sink() rpipe spawn Producer());";
    let diag=compile(&ACTOR.replace("%BODY%",body))
        .expect_err("directional ambiguity must fail before guest execution");
    assert_eq!(diag.code,"E_PIPE_DIRECTION");
}

#[test]
fn raise_panic_and_invalid_error_payloads_must_never_compile_as_trapped_none() {
    for line in [
        "val x = trap (raise Error(\"raise\"));",
        "val x = trap (panic Error(\"panic\"));",
        "raise Error(\"raise\");",
        "panic Error(\"panic\");",
        "throw 42;",
        "raise 42;",
        "panic 42;",
    ] {
        let source=format!("pub fnc main(): int {{ {line} return 0; }}");
        assert!(compile(&source).is_err(), "{line} must fail closed");
    }
}
