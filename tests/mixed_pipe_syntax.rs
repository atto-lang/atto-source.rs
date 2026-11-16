//! Mixed `pipe` parser and typed-actor preflight conformance.
//!
//! Parsing is not executable runtime proof: channel, stream, generator
//! capabilities, ownership/backpressure and the guest forwarding ABI remain
//! unsupported. No successful guest Wasm is allowed for a mixed pipeline.

use atto_compiler::ast::{ExprKind, PipeStage, Stmt};
use atto_compiler::{lexer, parser};

fn program(expression: &str, validate_payload: &str) -> String {
    format!(
        "define actor Parse as\n\
           receive(ActorMail<int> mail): void {{ emit(mail.value); }}\n\
         end\n\
         define actor Validate as\n\
           receive(ActorMail<{validate_payload}> mail): void {{ emit(mail.value); }}\n\
         end\n\
         define actor Sink as\n\
           receive(ActorMail<int> mail): void {{ }}\n\
         end\n\
         define actor Empty as\n\
           receive(ActorMail<int> mail): void {{ }}\n\
         end\n\
         pub fnc main(): int {{\n\
           val source: int = 0;\n\
           val sink: int = 0;\n\
           val connection = {expression};\n\
           return 0;\n\
         }}\n"
    )
}

fn parsed_pipe(expression: &str) -> Vec<PipeStage> {
    let tokens = lexer::lex(&program(expression, "int")).unwrap();
    let parsed = parser::Parser::new(tokens).parse_program().unwrap();
    let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &main.body[2] else {
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
        "spawn Parse() pipe sink pipe spawn Sink()",
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
        "spawn Parse() pipe sink pipe spawn Sink()",
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
