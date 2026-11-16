//! Pipeline failure-boundary regression. The source compiler has real
//! ordinary throw+trap and a restricted Option/Result wasm guest subset,
//! but mixed continuous pipe transport is intentionally NOT executable.
use atto_compiler::{compile, lexer, parser::Parser};

#[test]
fn existing_throw_to_none_and_result_err_to_some_err_still_compile() {
    // These existing guest programs exercise the actual ordinary-error
    // Wasm parser/checker/emitter, not an invented pipe runtime.
    for program in [
        include_str!("fixtures/pass/100-caller-trap-result-err.atto"),
        include_str!("fixtures/pass/101-throw-error-trap-none.atto"),
        include_str!("fixtures/pass/95-result-propagate-ok.atto"),
    ] {
        let bytes=compile(program).expect("existing ordinary failure path compiles");
        assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(bytes,compile(program).unwrap());
    }
}

#[test]
fn ordinary_error_trap_cannot_fake_a_pipe_guest_or_swallow_bad_wiring() {
    let prelude=r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void { emit(mail.value); }
end
pub fnc main(): int {
    val pipeline = trap (spawn Producer() pipe Stream.port<bool>(2).sink());
    return 0;
}
"#;
    let diagnostic=compile(prelude).expect_err("bad pipe must not become Some(..) or None");
    assert_eq!(diagnostic.code,"E_TYPE","{diagnostic}");

    for direction in [
        "spawn Producer() pipe Stream.port<int>(2).sink()",
        "Stream.port<int>(2).sink() rpipe spawn Producer()",
    ] {
        let program=prelude.replace(
            "spawn Producer() pipe Stream.port<bool>(2).sink()", direction);
        let diagnostic=compile(&program).expect_err("no executable pipe transport under trap");
        assert_eq!(diagnostic.code,"E_UNSUPPORTED","{direction}: {diagnostic}");
    }
}

#[test]
fn trap_and_question_mark_do_not_implicitly_handle_async_pipe_completion() {
    let template=r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void { emit(mail.value); }
end
pub fnc main(): int {
    %BODY%
    return 0;
}
"#;
    for stmt in [
        "val output = trap (Stream.port<int>(2).sink() rpipe spawn Producer());",
        "val output = (Stream.port<int>(2).sink() rpipe spawn Producer())?;",
        "val output = trap (spawn Producer() pipe Stream.port<int>(2).sink());",
    ] {
        let source=template.replace("%BODY%",stmt);
        let diag=compile(&source).expect_err("no fake checked completion before runtime ABI");
        // Type errors are also permitted if ? is used in an incompatible int
        // return context, but NO compiler output is authorized.
        assert!(matches!(diag.code,"E_UNSUPPORTED"|"E_TYPE"|"E_EFFECT"|"E_PIPE_CONTRACT"),
            "{stmt}: {diag}");
    }
}

#[test]
fn raise_and_panic_are_not_converted_into_option_by_trap() {
    for stmt in [
        "val x = trap (raise Error(\"fatal\"));",
        "val x = trap (panic Error(\"fatal\"));",
        "raise Error(\"fatal\");",
        "panic Error(\"fatal\");",
    ] {
        let source=format!("pub fnc main(): int {{ {stmt} return 0; }}");
        // These guest effects require the actor/runtime failure integration;
        // until supplied the browser compiler cannot emit a fake Option.None.
        assert!(compile(&source).is_err(),"{stmt} must not emit a Wasm guest");
    }
}

#[test]
fn option_result_values_are_not_a_pipeline_capability_by_themselves() {
    let source=r#"
pub fnc main(): int {
    val outcome: Result<int,string> = Err("not a PipeSource");
    val connection = outcome rpipe Stream.port<int>(1).sink();
    return 0;
}
"#;
    // No conversion of Err to a pipe error, stream event or None.
    assert!(compile(source).is_err());
}

#[test]
fn unknown_rpipe_direction_cannot_be_hidden_under_a_nested_trap() {
    let source=r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void { emit(mail.value); }
end
pub fnc main(): int {
    val maybe = trap (spawn Producer() pipe Stream.port<int>(1).sink()
        rpipe spawn Producer());
    return 0;
}
"#;
    let diag=compile(source).expect_err("no direction inference from trap");
    assert_eq!(diag.code,"E_PIPE_DIRECTION");
}

#[test]
fn actual_direction_is_recorded_in_ast_before_any_runtime_failure_conversion() {
    use atto_compiler::ast::{ExprKind,PipeDirection,Stmt};
    let src=r#"
define actor Source receives int emits int as
    receive(ActorMail<int> mail): void { emit(mail.value); }
end
pub fnc main(): int {
    val graph = Stream.port<int>(1).sink() rpipe spawn Source();
    return 0;
}
"#;
    let parsed=Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    let f=parsed.funcs.iter().find(|f|f.name=="main").unwrap();
    let Stmt::Bind{init,..}=&f.body[0] else{panic!("binding")};
    assert!(matches!(init.kind,ExprKind::DirectedPipeline(_,PipeDirection::RightToLeft)));
    assert_eq!(compile(src).unwrap_err().code,"E_UNSUPPORTED");
}
