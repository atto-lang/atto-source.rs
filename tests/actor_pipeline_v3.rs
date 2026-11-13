//! End-to-end source compilation assertions. The Node conformance harness
//! executes the resulting Wasm guest with a v3 actor-origin mailbox host.
//! No JavaScript interpretation of Atto source is permitted.
use atto_compiler::ast::{ExprKind, Stmt};
use atto_compiler::lexer::lex;
use atto_compiler::parser::Parser;

const SIMPLE: &str = include_str!("fixtures/pass/106-actor-pipeline-v3.atto");
const BOOL_PIPE: &str = include_str!("fixtures/pass/107-actor-pipeline-bool-v3.atto");

#[test]
fn both_aliases_have_identical_ordered_actor_stage_asts() {
    let p = Parser::new(lex(SIMPLE).unwrap())
        .parse_program()
        .unwrap();
    let main = p.funcs.iter().find(|f| f.name == "main").unwrap();
    let expected = ["Parse", "Validate", "Sink"];
    for stmt in &main.body[..2] {
        let Stmt::Bind { init, .. } = stmt else {
            panic!("expected actor pipeline binding");
        };
        let ExprKind::ActorPipeline(stages) = &init.kind else {
            panic!("actor pipeline AST was not created");
        };
        assert_eq!(stages, &expected);
    }
}

#[test]
fn emitted_actor_values_compile_to_deterministic_versioned_wasm() {
    for source in [SIMPLE, BOOL_PIPE] {
        let bytes = atto_compiler::compile(source).unwrap();
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
        assert_eq!(atto_compiler::compile(source).unwrap(), bytes);
        assert!(bytes.windows("atto.pipeline.v1".len()).any(|w| w == b"atto.pipeline.v1"));
        assert!(bytes.windows("actor_link".len()).any(|w| w == b"actor_link"));
        assert!(bytes.windows("actor_emit".len()).any(|w| w == b"actor_emit"));
    }
}

fn error_code(source: &str) -> &'static str {
    atto_compiler::compile(source).unwrap_err().code
}

#[test]
fn wrong_emitted_type_cannot_enter_a_different_actor_receive_type() {
    assert_eq!(error_code(&SIMPLE.replacen("emit(mail.value + 1);", "emit(true);", 1)), "E_TYPE");
    assert_eq!(error_code(&SIMPLE.replacen("emit(mail.value + 1);", "emit(1); emit(true);", 1)), "E_TYPE");
    assert_eq!(error_code(&SIMPLE.replacen("spawn Parse() | spawn Validate()", "spawn Sink() | spawn Validate()", 1)), "E_TYPE");
}

#[test]
fn invalid_pipeline_shapes_and_unknown_actors_fail_closed() {
    assert_eq!(error_code(&SIMPLE.replacen("spawn Validate()", "spawn Missing()", 1)), "E_UNBOUND");
    assert_eq!(error_code(&SIMPLE.replacen("spawn Validate()", "42", 1)), "E_TYPE");
    assert_eq!(error_code("pub fnc main(): int { val x = 1 pipe 2; return 0; }"), "E_TYPE");
    assert_eq!(error_code("pub fnc main(): int { emit(41); return 0; }"), "E_UNSUPPORTED");
}

#[test]
fn v2_expression_operators_remain_distinct() {
    let c = atto_compiler::compile(
        "pub fnc main(): int { if true | false then return 1; fi return 0; }",
    )
    .unwrap();
    assert!(!c.windows("atto.pipeline.v1".len()).any(|w| w == b"atto.pipeline.v1"));
    let src = include_str!("fixtures/pass/93-functional-pipeline.atto");
    assert!(atto_compiler::compile(src).is_ok());
}
