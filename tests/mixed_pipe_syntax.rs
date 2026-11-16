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

#[test]
fn pure_source_actor_needs_explicit_executable_start_contract() {
    let source = "define actor OnlySource emits int as
        end
        pub fnc main(): int { return 0; }";
    let parsed = parser::Parser::new(lexer::lex(source).unwrap())
        .parse_program().unwrap();
    assert_eq!(parsed.actors[0].declared_emits, Some(atto_compiler::ast::Type::Int));
    let err = atto_compiler::compile(source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
    assert!(err.message.contains("startup producer hook"), "{err}");
}

#[test]
fn declared_input_cannot_claim_a_missing_receive_handler() {
    let source = "define actor NoSink receives int as
        end
        pub fnc main(): int { return 0; }";
    let err = atto_compiler::compile(source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("declared receives"), "{err}");
}

#[test]
fn actor_transform_may_emit_multiple_values_of_declared_type() {
    let source = program("source pipe middle pipe sink", "int")
        .replacen("emit(mail.value);", "emit(mail.value); emit(mail.value);", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
    assert!(err.message.contains("mixed pipe"), "{err}");
}

#[test]
fn stored_actor_refs_can_form_a_typed_mixed_graph_without_dynamic_casts() {
    let source = program("source pipe middle pipe sink", "int");
    let parsed = parser::Parser::new(lexer::lex(&source).unwrap())
        .parse_program().unwrap();
    let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &main.body[3] else { panic!("pipe binding"); };
    let ExprKind::MixedPipeline(stages) = &init.kind else { panic!("mixed pipeline"); };
    assert_eq!(stages.len(), 3);
    assert!(stages.iter().all(|stage| matches!(stage, PipeStage::Endpoint(_))));
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
}

#[test]
fn repeating_one_actor_ref_in_a_pipe_graph_requires_explicit_fanout() {
    let source = program("source pipe middle pipe source", "int");
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_PIPE_CONTRACT", "{err}");
    assert!(err.message.contains("more than once"), "{err}");
}

#[test]
fn a_class_instance_is_not_an_inferred_pipe_endpoint() {
    let source = program("source pipe middle", "int")
        .replacen("pub fnc main(): int {",
                  "define class Dummy as end\n pub fnc main(): int {", 1)
        .replacen("val source = spawn Parse();",
                  "val source = new Dummy();", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("PipeSource/PipeSink"), "{err}");
}

#[test]
fn known_actor_ref_edges_are_checked_across_three_stage_graph() {
    // The first edge is bool -> bool, but the second is bool -> int.
    // This must fail at the middle->sink edge, not only at stage one.
    let source = program("source pipe middle pipe sink", "bool")
        .replacen("define actor Parse receives int emits int",
                  "define actor Parse receives int emits bool", 1)
        .replacen("emit(mail.value);", "emit(true);", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("emitted"), "{err}");
}

#[test]
fn bounded_channel_stage_has_explicit_duplex_int_contract() {
    use atto_compiler::ast::{PipeEndpointKind, Type};
    let src = program("spawn Parse() pipe Channel<int>.bounded(2) pipe spawn Sink()", "int");
    let parsed = parser::Parser::new(lexer::lex(&src).unwrap()).parse_program().unwrap();
    let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &main.body[3] else { panic!("pipe binding"); };
    let ExprKind::MixedPipeline(stages) = &init.kind else { panic!("mixed pipe"); };
    assert_eq!(stages.len(), 3);
    match &stages[1] {
        PipeStage::Endpoint(expr) => assert!(matches!(
            &expr.kind,
            ExprKind::PipeEndpoint(PipeEndpointKind::ChannelBounded, Type::Int, 2)
        )),
        _ => panic!("expected channel port"),
    }
    let err = atto_compiler::compile(&src).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
    assert!(err.message.contains("mixed pipe"), "{err}");
}

#[test]
fn buffered_stream_adapter_is_a_typed_midstage_not_a_single_write() {
    let err = atto_compiler::compile(&program(
        "spawn Parse() pipe Stream.buffered<int>(0) pipe spawn Sink()", "int"
    )).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
    assert!(err.message.contains("mixed pipe"), "{err}");
}

#[test]
fn explicit_stream_port_sink_admits_matching_emission_only() {
    for expression in [
        "spawn Parse() pipe Stream.port<int>(8).sink()",
        "spawn Parse() pipe Channel<int>.bounded(8)",
    ] {
        let err = atto_compiler::compile(&program(expression, "int")).unwrap_err();
        assert_eq!(err.code, "E_UNSUPPORTED", "{expression}: {err}");
    }
}

#[test]
fn stream_sink_must_not_be_used_as_an_implicit_source() {
    let err = atto_compiler::compile(&program(
        "spawn Parse() pipe Stream.port<int>(2).sink() pipe spawn Sink()", "int"
    )).unwrap_err();
    assert_eq!(err.code, "E_TYPE", "{err}");
    assert!(err.message.contains("PipeSource"), "{err}");
}

#[test]
fn actor_and_channel_element_type_mismatch_is_statically_rejected() {
    for expression in [
        "spawn Parse() pipe Channel<bool>.bounded(2)",
        "spawn Parse() pipe Stream.buffered<bool>(2) pipe spawn Sink()",
        "spawn Parse() pipe Stream.port<bool>(2).sink()",
    ] {
        let err = atto_compiler::compile(&program(expression, "int")).unwrap_err();
        assert_eq!(err.code, "E_TYPE", "{expression}: {err}");
        assert!(err.message.contains("emitted"), "{expression}: {err}");
    }
}

#[test]
fn literal_channel_and_stream_capacity_is_bounded_and_nonnegative() {
    for (expression, want) in [
        ("spawn Parse() pipe Channel<int>.bounded(4097)", "E_SOURCE_LIMIT"),
        ("spawn Parse() pipe Channel<int>.bounded(-1)", "E_PIPE_CONTRACT"),
        ("spawn Parse() pipe Stream.buffered<int>(true)", "E_PIPE_CONTRACT"),
        ("spawn Parse() pipe Stream.port<int>(2).sink(1)", "E_PIPE_CONTRACT"),
    ] {
        let err = atto_compiler::compile(&program(expression, "int")).unwrap_err();
        assert_eq!(err.code, want, "{expression}: {err}");
    }
}

#[test]
fn stream_readable_view_must_not_implicitly_be_a_writable_sink() {
    let err = atto_compiler::compile(&program(
        "spawn Parse() pipe Stream.port<int>(2).stream()", "int"
    )).unwrap_err();
    assert_eq!(err.code, "E_PIPE_CONTRACT", "{err}");
}

#[test]
fn unsupported_stream_item_type_rejects_before_guest_allocation() {
    let err = atto_compiler::compile(&program(
        "spawn Parse() pipe Stream.buffered<string>(2)", "int"
    )).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED_TYPE", "{err}");
}

#[test]
fn standalone_channel_constructor_is_not_a_guest_allocation_yet() {
    let source = program("spawn Parse() pipe sink", "int")
        .replacen("val connection = spawn Parse() pipe sink;",
                  "val connection = Channel<int>.bounded(2);", 1);
    let err = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");
    assert!(err.message.contains("owned guest transport"), "{err}");
}
