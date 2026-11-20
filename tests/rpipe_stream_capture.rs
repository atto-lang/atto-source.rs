//! Frontend-only admission for direction-explicit continuous pipelines.
//! No mixed-channel/stream guest ABI, PipeHandle or checked failure runtime
//! exists here. Only parser and static rejection behavior can be asserted.
use atto_compiler::ast::{ExprKind, PipeDirection, PipeEndpointKind, PipeStage, Stmt, Type};
use atto_compiler::{compile, lexer, parser::Parser};

fn source(graph: &str) -> String {
    format!(r#"
define actor Producer receives int emits int as
    receive(ActorMail<int> mail): void {{ emit(mail.value); }}
end
define actor Transform receives int emits int as
    receive(ActorMail<int> mail): void {{ emit(mail.value); }}
end
define actor Consumer receives int as
    receive(ActorMail<int> mail): void {{ }}
end
pub fnc main(): int {{
    val connection = {graph};
    return 0;
}}
"#)
}

fn parsed(graph: &str) -> ExprKind {
    let p=Parser::new(lexer::lex(&source(graph)).unwrap()).parse_program().unwrap();
    let main=p.funcs.iter().find(|f|f.name=="main").unwrap();
    let Stmt::Bind {init,..}=&main.body[0] else {panic!("expected pipeline binding")};
    init.kind.clone()
}

fn error(graph: &str) -> String {
    let diag=compile(&source(graph)).expect_err("no pipe guest ABI; no compiled artifact");
    diag.code.to_owned()
}

#[test]
fn reverse_pipeline_keeps_lexical_order_and_declared_direction() {
    let a=parsed("spawn Producer() pipe spawn Transform() pipe spawn Consumer()");
    assert!(matches!(a,ExprKind::ActorPipeline(stages)
        if stages==["Producer","Transform","Consumer"]));
    let b=parsed("spawn Consumer() rpipe spawn Transform() rpipe spawn Producer()");
    let ExprKind::DirectedPipeline(stages,PipeDirection::RightToLeft)=b
        else {panic!("rpipe must retain explicit direction metadata")};
    let order:Vec<_>=stages.iter().map(|stage|match stage {
        PipeStage::Actor(n)=>n.as_str(),
        _=>panic!("unexpected non-actor stage"),
    }).collect();
    assert_eq!(order,vec!["Consumer","Transform","Producer"]);
    assert_eq!(error("spawn Consumer() rpipe spawn Transform() rpipe spawn Producer()"),"E_UNSUPPORTED");
}

#[test]
fn reverse_pipeline_statics_agree_with_forward_order() {
    for graph in [
        "spawn Producer() pipe Stream.buffered<int>(1) pipe spawn Consumer()",
        "spawn Consumer() rpipe Stream.buffered<int>(1) rpipe spawn Producer()",
        "spawn Producer() pipe Channel<int>.bounded(0) pipe spawn Consumer()",
        "spawn Consumer() rpipe Channel<int>.bounded(0) rpipe spawn Producer()",
        "spawn Producer() pipe Stream.port<int>(8).sink()",
        "Stream.port<int>(8).sink() rpipe spawn Producer()",
        "Stream.port<int>(8).stream() pipe spawn Consumer()",
        "spawn Consumer() rpipe Stream.port<int>(8).stream()",
    ] {
        assert_eq!(error(graph),"E_UNSUPPORTED","{graph}");
    }
}

#[test]
fn inline_stream_capture_is_a_terminal_bounded_sink_in_either_direction() {
    for graph in [
        "spawn Producer() pipe (let results: Stream<int> = Stream.buffered<int>(64))",
        "(let results: Stream<int> = Stream.buffered<int>(64)) rpipe spawn Producer()",
        "spawn Producer() pipe spawn Transform() pipe (val results: Stream<int> = Stream.buffered<int>(0))",
        "(val results: Stream<int> = Stream.buffered<int>(0)) rpipe spawn Transform() rpipe spawn Producer()",
    ] {
        assert_eq!(error(graph),"E_UNSUPPORTED","{graph}");
        let ast=parsed(graph);
        let stages=match ast {
            ExprKind::MixedPipeline(v) | ExprKind::DirectedPipeline(v,_)=>v,
            x=>panic!("capture must be a graph: {x:?}"),
        };
        let captures:Vec<_>=stages.iter().filter_map(|stage|match stage {
            PipeStage::Endpoint(e) => match &e.kind {
                ExprKind::PipeCapture(name,ty,capacity,mutable)=>
                    Some((name.as_str(),*ty,*capacity,*mutable)),
                _=>None,
            },
            _=>None,
        }).collect();
        assert_eq!(captures.len(),1);
        assert_eq!(captures[0].0,"results");
        assert_eq!(captures[0].1,Type::Int);
    }
}

#[test]
fn reverse_wiring_rejects_missing_capabilities_and_mismatches() {
    for (graph,expected) in [
        ("spawn Producer() rpipe spawn Consumer()","E_TYPE"),
        ("spawn Producer() pipe Stream.port<int>(1).stream()","E_TYPE"),
        ("Stream.port<int>(1).stream() rpipe spawn Producer()","E_TYPE"),
        ("spawn Producer() pipe Stream.port<bool>(1).sink()","E_TYPE"),
        ("Stream.port<bool>(1).sink() rpipe spawn Producer()","E_TYPE"),
        ("spawn Producer() pipe (let results: Stream<bool> = Stream.buffered<bool>(1))","E_TYPE"),
        ("(let results: Stream<bool> = Stream.buffered<bool>(1)) rpipe spawn Producer()","E_TYPE"),
        ("(let results: Stream<int> = Stream.buffered<int>(1)) pipe spawn Consumer()","E_TYPE"),
        ("spawn Producer() rpipe (let results: Stream<int> = Stream.buffered<int>(1))","E_PIPE_CONTRACT"),
    ] {
        assert_eq!(error(graph),expected,"{graph}");
    }
}

#[test]
fn inline_declaration_rejects_const_unbounded_mismatched_and_bad_constructor() {
    for (graph,expected) in [
        ("spawn Producer() pipe (const results: Stream<int> = Stream.buffered<int>(3))","E_CONST_EFFECT"),
        ("spawn Producer() pipe (let results: Stream<int> = Stream.buffered<bool>(3))","E_TYPE"),
        ("spawn Producer() pipe (let results: Stream<int> = Stream.buffered<int>(4097))","E_SOURCE_LIMIT"),
        ("spawn Producer() pipe (let results: Stream<int> = Stream.buffered<int>(-1))","E_PIPE_CONTRACT"),
        ("spawn Producer() pipe (let results: Stream<int> = Stream.port<int>(1))","E_PIPE_CONTRACT"),
        ("spawn Producer() pipe (let results: Stream<int> = Stream.buffered<int>(true))","E_PIPE_CONTRACT"),
        ("spawn Producer() pipe (let results: Stream<string> = Stream.buffered<string>(3))","E_UNSUPPORTED_TYPE"),
    ] {
        assert_eq!(error(graph),expected,"{graph}");
    }
}

#[test]
fn direction_is_not_inferred_by_endpoint_duplex_type() {
    for (graph,expected) in [
        ("spawn Producer() pipe Channel<int>.bounded(2) rpipe spawn Consumer()","E_PIPE_DIRECTION"),
        ("spawn Consumer() rpipe Channel<int>.bounded(2) pipe spawn Producer()","E_PIPE_DIRECTION"),
        ("spawn Consumer() rpipe 3","E_TYPE"),
        ("3 rpipe spawn Producer()","E_TYPE"),
    ] {
        assert_eq!(error(graph),expected,"{graph}");
    }
}

#[test]
fn inline_capture_outside_graph_and_multi_stage_mid_capture_are_rejected() {
    for graph in [
        "(let results: Stream<int> = Stream.buffered<int>(64))",
        "spawn Producer() pipe (let results: Stream<int> = Stream.buffered<int>(64)) pipe spawn Consumer()",
        "spawn Consumer() rpipe (let results: Stream<int> = Stream.buffered<int>(64)) rpipe spawn Producer()",
    ] {
        assert_eq!(error(graph),"E_PIPE_CONTRACT","{graph}");
    }
}

#[test]
fn parser_does_not_reinterpret_single_value_and_functional_operators_as_rpipe() {
    use atto_compiler::lexer::{lex,Tok};
    let tokens=lex("A pipe B; B rpipe A; a |> f; f <| a; sink << one; source >> val one;").unwrap();
    // pipe/rpipe remain distinct contextual keywords, ordinary symbol
    // operators remain independent tokens; neither is a stream pump.
    assert!(tokens.iter().any(|t| matches!(&t.tok,Tok::Ident(w) if w=="pipe")));
    assert!(tokens.iter().any(|t| matches!(&t.tok,Tok::Ident(w) if w=="rpipe")));
    assert!(tokens.iter().any(|t| matches!(&t.tok,Tok::PipeGt)));
    assert!(tokens.iter().any(|t| matches!(&t.tok,Tok::Lt)));
    assert!(tokens.iter().any(|t| matches!(&t.tok,Tok::Gt)));
    // The mistakenly reversed symbol '|<' has NOT been adopted.
    assert!(compile("pub fnc main(): int { return 2 |< 1; }").is_err());
}

#[test]
fn stream_capture_ast_metadata_includes_capacity_and_mutability() {
    let graph="(let results: Stream<int> = Stream.buffered<int>(4)) rpipe Stream.port<int>(4).stream()";
    let parsed=parsed(graph);
    let ExprKind::DirectedPipeline(stages,PipeDirection::RightToLeft)=parsed
        else {panic!("expected reversed graph")};
    assert_eq!(stages.len(),2);
    assert!(matches!(&stages[0],PipeStage::Endpoint(e)
        if matches!(&e.kind,ExprKind::PipeCapture(name,Type::Int,4,true)
            if name=="results")));
    assert!(matches!(&stages[1],PipeStage::Endpoint(e)
        if matches!(&e.kind,ExprKind::PipeEndpoint(PipeEndpointKind::StreamReadable,Type::Int,4))));
    assert_eq!(error(graph),"E_UNSUPPORTED");
}
/// Named ActorRef endpoints are not fresh actor constructors: the graph must
/// preserve references and typed direction, never infer a reverse mailbox.
/// These are FRONTEND TESTS ONLY; no guest transport ABI is supplied.
fn named_actor_program(statement: &str, producer_emits: bool) -> String {
    let output_clause = if producer_emits { " emits int" } else { "" };
    format!(r#"
define actor Producer receives int{output_clause} as
    receive(ActorMail<int> mail): void {{ {emit} }}
end
define actor Consumer receives int as
    receive(ActorMail<int> mail): void {{ }}
end
pub fnc main(): int {{
    val producer = spawn Producer();
    val consumer = spawn Consumer();
    {statement}
    return 0;
}}
"#, emit = if producer_emits { "emit(mail.value);" } else { "" })
}

fn named_graph_kind(statement: &str) -> ExprKind {
    let source = named_actor_program(statement, true);
    let program = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let main = program.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init, .. } = &main.body[2] else {
        panic!("expected third statement to bind a checked named actor connection");
    };
    init.kind.clone()
}

#[test]
fn named_actor_references_preserve_directional_wiring() {
    let forward = named_graph_kind("val forward = producer pipe consumer;");
    let reverse = named_graph_kind("val reverse = consumer rpipe producer;");
    let ExprKind::MixedPipeline(forward_stages) = forward else {
        panic!("named actor pipe must remain a mixed endpoint graph");
    };
    let ExprKind::DirectedPipeline(reverse_stages, PipeDirection::RightToLeft) = reverse else {
        panic!("named actor rpipe must retain reverse direction");
    };
    let names = |stages: Vec<PipeStage>| -> Vec<String> {
        stages.iter().map(|stage| match stage {
            PipeStage::Endpoint(atto_compiler::ast::Expr { kind: ExprKind::Var(name), .. }) => name.clone(),
            other => panic!("expected named ActorRef endpoint, got {other:?}"),
        }).collect()
    };
    assert_eq!(names(forward_stages), ["producer", "consumer"]);
    assert_eq!(names(reverse_stages), ["consumer", "producer"]);
    for statement in [
        "val forward = producer pipe consumer;",
        "val reverse = consumer rpipe producer;",
    ] {
        let diagnostic = compile(&named_actor_program(statement, true))
            .expect_err("typed graph has no executable transport yet");
        assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{statement}: {diagnostic}");
    }
}

#[test]
fn named_actor_wrong_direction_and_missing_emit_are_rejected_before_codegen() {
    // The user's original Producer receives int but never declares emits:
    // receiving an ActorMail<int> is not proof of PipeSource<int>.
    for statement in [
        "val forward = producer pipe consumer;",
        "val reverse = consumer rpipe producer;",
    ] {
        let diagnostic = compile(&named_actor_program(statement, false))
            .expect_err("receives-only actor cannot be a source");
        assert_eq!(diagnostic.code, "E_TYPE", "{statement}: {diagnostic}");
    }
    for statement in [
        "val backwards = consumer pipe producer;",
        "val backwards = producer rpipe consumer;",
    ] {
        let diagnostic = compile(&named_actor_program(statement, true))
            .expect_err("reverse topology cannot infer emissions on sink-only Consumer");
        assert_eq!(diagnostic.code, "E_TYPE", "{statement}: {diagnostic}");
    }
}

#[test]
fn two_spelling_variants_do_not_claim_a_duplex_connection() {
    let source = named_actor_program(
        "val first = producer pipe consumer;\n    val second = consumer rpipe producer;",
        true,
    );
    let program = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let main = program.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { init: first, .. } = &main.body[2] else { panic!("first graph"); };
    let Stmt::Bind { init: second, .. } = &main.body[3] else { panic!("second graph"); };
    assert!(matches!(&first.kind, ExprKind::MixedPipeline(_)));
    assert!(matches!(&second.kind,
        ExprKind::DirectedPipeline(_, PipeDirection::RightToLeft)));
    // They are two graph-construction expressions, both producer -> consumer.
    // Duplicate edge admission requires a runtime fan-out/ownership policy;
    // no unsupported graph may be emitted as an executable Wasm artifact.
    assert_eq!(compile(&source).unwrap_err().code, "E_UNSUPPORTED");
}
