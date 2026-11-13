//! Frontend-only proof of actor pipe AST and receive-type admission.
//! A successful parse is NOT a runnable guest. Until the actor-origin
//! forwarding ABI exists, compile() must return E_UNSUPPORTED and no Wasm.
use atto_compiler::ast::{ExprKind, Stmt};
use atto_compiler::lexer::lex;
use atto_compiler::parser::Parser;

fn program(expression: &str, middle_type: &str) -> String {
    format!(
        "define actor Parse as\n\
           receive(ActorMail<int> mail): void {{ return; }}\n\
         end\n\
         define actor Validate as\n\
           receive(ActorMail<{middle_type}> mail): void {{ return; }}\n\
         end\n\
         define actor Sink as\n\
           receive(ActorMail<int> mail): void {{ return; }}\n\
         end\n\
         pub fnc main(): int {{\n\
           val pipeline = {expression};\n\
           return 0;\n\
         }}\n"
    )
}

#[test]
fn both_spellings_and_mixed_aliases_build_the_same_ordered_ast() {
    for expression in [
        "spawn Parse() | spawn Validate() | spawn Sink()",
        "spawn Parse() pipe spawn Validate() pipe spawn Sink()",
        "spawn Parse() | spawn Validate() pipe spawn Sink()",
    ] {
        let source = program(expression, "int");
        let parsed = Parser::new(lex(&source).unwrap()).parse_program().unwrap();
        let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
        let Stmt::Bind { init, .. } = &main.body[0] else {
            panic!("expected pipeline binding");
        };
        match &init.kind {
            ExprKind::ActorPipeline(stages) => {
                assert_eq!(stages, &["Parse", "Validate", "Sink"]);
            }
            other => panic!("unexpected AST node: {other:?}"),
        }
        let error = atto_compiler::compile(&source).unwrap_err();
        assert_eq!(error.code, "E_UNSUPPORTED", "{expression}: {error}");
        assert!(error.message.contains("actor pipeline"), "{error}");
    }
}

#[test]
fn actor_pipe_rejects_invalid_topology_before_codegen() {
    for (expression, middle_type, want) in [
        ("spawn Parse() pipe spawn Validate() pipe spawn Sink()", "bool", "E_TYPE"),
        ("spawn Parse() | spawn Missing()", "int", "E_UNBOUND"),
        ("spawn Parse() pipe 42", "int", "E_TYPE"),
        ("1 pipe 2", "int", "E_TYPE"),
        ("1 | 2", "int", "E_UNSUPPORTED"),
    ] {
        let source = program(expression, middle_type);
        let error = atto_compiler::compile(&source).unwrap_err();
        assert_eq!(error.code, want, "{expression}: {error}");
    }
}

#[test]
fn stage_limit_is_bounded_and_cannot_allocate_a_guest() {
    let stages = std::iter::repeat("spawn Parse()")
        .take(17)
        .collect::<Vec<_>>()
        .join(" pipe ");
    let error = atto_compiler::compile(&program(&stages, "int")).unwrap_err();
    assert_eq!(error.code, "E_SOURCE_LIMIT");
}

#[test]
fn legacy_condition_pipe_stays_boolean_or_not_actor_composition() {
    let source = "pub fnc main(): int { if true | false then return 1; fi return 0; }";
    let wasm = atto_compiler::compile(source).unwrap();
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
}
