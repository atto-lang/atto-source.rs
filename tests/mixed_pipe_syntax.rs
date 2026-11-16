//! Mixed `pipe` parser and typed-actor preflight conformance.
//!
//! Parsing is not executable runtime proof: channel, stream, generator
//! capabilities, ownership/backpressure and the guest forwarding ABI remain
//! unsupported. No successful guest Wasm is allowed for a mixed pipeline.

use atto_compiler::ast::{ExprKind, PipeStage, Stmt};
use atto_compiler::{lexer, parser};

fn program(expression: &str, validate_payload: &str) -> String {
    format!(
        "define actor Parse receives int emits int as\n\
           receive(ActorMail<int> mail): void {{ emit(mail.value); }}\n\
         end\n\
         define actor Validate receives {validate_payload} emits {validate_payload} as\n\
           receive(ActorMail<{validate_payload}> mail): void {{ emit(mail.value); }}\n\
         end\n\
         define actor Sink receives int as\n\
           receive(ActorMail<int> mail): void {{ }}\n\
         end\n\
         define actor Empty receives int as\n\
           receive(ActorMail<int> mail): void {{ }}\n\
         end\n\
         pub fnc main(): int {{\n\
           val source = spawn Parse();\n\
           val sink = spawn Sink();\n\
           val middle = spawn Validate();\n\\
           val connection = {expression};\n\
           return 0;\n\
         }}\n"
    )
}

fn parsed_pipe(expression: &str) -> Vec<PipeStage> {
    let tokens = lexer::lex(&program(expression, "int")).unwrap();
    let parsed = parser::Parser::new(tokens).parse_program().unwrap();
    let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &main.body[3] else {
        panic!("expected pipeline binding");
    };
    let ExprKind::MixedPipeline(stages) = &init.kind else {
        panic!("expected mixed endpoint AST, got {:?}", init.kind);
    };
    stages.clone()
}

#[test]
fn mixed_pipe_ast_preserves_endpoint_order_and_actor_roles() {
    for expression in [
        "spawn Parse() pipe sink",
        "source pipe spawn Sink()",
        "spawn Parse() pipe middle pipe spawn Sink()",
        "source pipe spawn Validate() pipe sink",
        "spawn Parse() | spawn Validate() pipe sink",
    ] {
        let stages = parsed_pipe(expression);
        assert!(stages.len() >= 2, "{expression}");
        assert!(stages.iter().any(|s| matches!(s, PipeStage::Actor(_))), "{expression}");
        assert!(stages.iter().any(|s| matches!(s, PipeStage::Endpoint(_))), "{expression}");
        for stage in &stages {
            match stage {
                PipeStage::Actor(name) => assert!(!name.is_empty()),
                PipeStage::Endpoint(expr) => assert!(
                    matches!(&expr.kind, ExprKind::Var(_) | ExprKind::Call(_, _)
                        | ExprKind::Member(_, _) | ExprKind::MethodCall(_, _, _)),
                    "{expression}"
                ),
            }
        }
    }
}

#[test]
fn mixed_pipe_compiler_fails_closed_without_guest_abi() {
    for expression in [
        "spawn Parse() pipe sink",
        "source pipe spawn Sink()",
        "spawn Parse() pipe middle pipe spawn Sink()",
        "spawn Parse() pipe spawn Validate() pipe sink",
    ] {
        let err = atto_compiler::compile(&program(expression, "int")).unwrap_err();
        assert_eq!(err.code, "E_UNSUPPORTED", "{expression}: {err}");
        assert!(err.message.contains("mixed pipe"), "{expression}: {err}");
    }
}

#[test]
fn mixed_pipe_rejects_invalid_literal_endpoints_and_legacy_bar() {
    for expression in [
        "spawn Parse() pipe 42",
        "42 pipe spawn Sink()",
        "spawn Parse() | sink",
    ] {
        let err = atto_compiler::compile(&program(expression, "int")).unwrap_err();
        assert_eq!(err.code, "E_TYPE", "{expression}: {err}");
    }
}

#[test]
fn mixed_pipe_preserves_actor_preflight_errors() {
    for (expression, payload, wanted) in [
        ("spawn Parse() pipe spawn Validate() pipe sink", "bool", "E_TYPE"),
        ("spawn Missing() pipe sink", "int", "E_UNBOUND"),
        ("spawn Empty() pipe sink", "int", "E_TYPE"),
    ] {
        let err = atto_compiler::compile(&program(expression, payload)).unwrap_err();
        assert_eq!(err.code, wanted, "{expression}: {err}");
    }
}

#[test]
fn mixed_pipe_stage_budget_is_bounded() {
    let chain = std::iter::once("spawn Parse()".to_string())
        .chain(std::iter::repeat("pipe sink".to_string()).take(16))
        .collect::<Vec<_>>()
        .join(" ");
    // 17 total stages exceeds the existing 16-stage tenant cap.
    let err = atto_compiler::compile(&program(&chain, "int")).unwrap_err();
    assert_eq!(err.code, "E_SOURCE_LIMIT", "{err}");
}

#[test]
fn actor_headers_derive_exact_typed_roles_in_ast() {
    let source = program("spawn Parse() pipe sink", "int");
    let parsed = parser::Parser::new(lexer::lex(&source).unwrap())
        .parse_program().unwrap();
    let parse = parsed.actors.iter().find(|a| a.name == "Parse").unwrap();
    let validate = parsed.actors.iter().find(|a| a.name == "Validate").unwrap();
    let sink = parsed.actors.iter().find(|a| a.name == "Sink").unwrap();
    assert_eq!(parse.declared_receives, Some(atto_compiler::ast::Type::Int));
    assert_eq!(parse.declared_emits, Some(atto_compiler::ast::Type::Int));
    assert_eq!(validate.declared_receives, Some(atto_compiler::ast::Type::Int));
    assert_eq!(validate.declared_emits, Some(atto_compiler::ast::Type::Int));
    assert_eq!(sink.declared_receives, Some(atto_compiler::ast::Type::Int));
    assert_eq!(sink.declared_emits, None);
}

#[test]
fn mixed_actor_source_must_declare_emits_even_when_body_calls_emit() {
    let source = program("spawn Parse() pipe sink", "int")
        .replacen("define actor Parse receives int emits int",
                  "define actor Parse receives int", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("explicit emits"), "{err}");
}

#[test]
fn mixed_actor_destination_must_declare_receives_even_with_mail_handler() {
    let source = program("source pipe spawn Sink()", "int")
        .replacen("define actor Sink receives int", "define actor Sink", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("explicit receives"), "{err}");
}

#[test]
fn declared_receive_type_must_match_actual_actor_mail_handler() {
    let source = program("spawn Parse() pipe spawn Validate() pipe sink", "int")
        .replacen("define actor Validate receives int emits int",
                  "define actor Validate receives bool emits int", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("declared receives"), "{err}");
}

#[test]
fn declared_emit_type_must_match_actual_emitted_values() {
    let source = program("spawn Parse() pipe sink", "int")
        .replacen("define actor Parse receives int emits int",
                  "define actor Parse receives int emits bool", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("declares emits"), "{err}");
}

#[test]
fn duplicate_or_reordered_actor_pipe_clauses_are_rejected() {
    for header in [
        "define actor Parse emits int receives int as",
        "define actor Parse receives int receives int as",
        "define actor Parse receives int emits int emits int as",
    ] {
        let source = program("spawn Parse() pipe sink", "int")
            .replacen("define actor Parse receives int emits int as", header, 1);
        let err = atto_compiler::compile(&source).unwrap_err();
        assert_eq!(err.code, "E_PIPE_CONTRACT", "{header}: {err}");
    }
}

#[test]
fn a_non_emitting_handler_cannot_claim_a_pipe_output() {
    let source = program("spawn Parse() pipe sink", "int")
        .replacen("emit(mail.value);", "return;", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("declares emits"), "{err}");
}

#[test]
fn actor_only_legacy_pipeline_still_has_a_migration_path() {
    let source = program("spawn Parse() pipe spawn Validate()", "int")
        .replace(" receives int emits int as", " as");
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
    assert!(err.message.contains("actor pipeline"), "{err}");
}

#[test]
fn scalar_endpoint_is_rejected_before_runtime_abi_admission() {
    let source = program("source pipe spawn Sink()", "int")
        .replacen("val source = spawn Parse();", "val source: int = 0;", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("scalar"), "{err}");
}

#[test]
fn unbound_endpoint_is_not_accepted_as_a_channel_by_spelling() {
    let source = program("source pipe spawn Sink()", "int")
        .replacen("val source = spawn Parse();", "val another = spawn Parse();", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_UNBOUND", "{err}");
    assert!(err.message.contains("unknown pipe endpoint"), "{err}");
}

#[test]
fn actor_ref_sink_cannot_be_used_as_middle_transform_stage() {
    let err = atto_compiler::compile(&program(
        "spawn Parse() pipe sink pipe spawn Sink()", "int"
    )).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("explicit emits"), "{err}");
}

#[test]
fn actor_ref_source_is_checked_as_readable_not_duplex_by_default() {
    let err = atto_compiler::compile(&program(
        "sink pipe spawn Sink()", "int"
    )).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("explicit emits"), "{err}");
}

#[test]
fn direct_mixed_actor_ref_edges_check_emitted_payload_types() {
    let source = program("source pipe spawn Validate() pipe sink", "bool");
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("emitted"), "{err}");
}
