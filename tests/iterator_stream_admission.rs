//! Typed lazy iterator stage admission; no guest runtime or AOT claim.
use atto_compiler::diag::Pos;
use atto_compiler::iterator_stream_admission::{
    inspect, CallbackMode, Family, PipelinePlan, Stage,
};
use atto_compiler::lexer::lex;

fn pos() -> Pos { Pos { line: 1, col: 1 } }

fn map(input: &str, output: &str, mode: CallbackMode, consuming_proven: bool) -> Stage {
    Stage::Map {
        input: input.into(), output: output.into(),
        mode, consuming_proven,
    }
}

fn filter(input: &str, result: &str, mode: CallbackMode) -> Stage {
    Stage::Filter { input: input.into(), result: result.into(), mode }
}

#[test]
fn lexer_handles_protocol_types_without_matching_quoted_text_or_comments() {
    let candidates = inspect(&lex(r#"
        // Stream<int> and Iterator<void> are just comments.
        val x = "AsyncIterator<int> and Iterator<bool>";
        val Stream<int> s = source;
        val Iterator<Option<int>> cursor = other;
    "#).unwrap()).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].family, Family::Stream);
    assert_eq!(candidates[1].family, Family::Iterator);
    assert!(candidates[0].element_span.0 < candidates[0].element_span.1);
    assert!(candidates[1].element_span.0 < candidates[1].element_span.1);
}

#[test]
fn malformed_and_void_protocol_generics_fail_before_guest_emission() {
    for src in [
        "Iterator<>", "Stream<int,int>", "AsyncIterator<void>",
        "Iterable<never>", "Stream<int", "Iterator<int,>", "Stream<Iterator<int",
    ] {
        let err = inspect(&lex(src).unwrap()).expect_err(src);
        assert_eq!(err.code, "E_STREAM_TYPE", "{src}: {err}");
    }
}

#[test]
fn other_generic_names_and_non_generic_methods_are_not_reclassified() {
    let tokens = lex("val a = Array<int>.of(42); val b = value.map(inc);").unwrap();
    assert!(inspect(&tokens).unwrap().is_empty());
}

#[test]
fn browser_compiler_rejects_valid_protocol_types_instead_of_faking_wasm() {
    for src in [
        "pub fnc main(): int { val Stream<int> stream = source; return 42; }",
        "pub fnc main(): int { val Iterator<int> iter = source; return 42; }",
        "pub fnc main(): int { val AsyncIterator<int> iter = source; return 42; }",
        "pub fnc main(): int { val Iterable<int> iter = source; return 42; }",
    ] {
        let err = atto_compiler::compile(src).expect_err(src);
        assert_eq!(err.code, "E_UNSUPPORTED_TYPE", "{src}: {err}");
    }
    let valid = atto_compiler::compile("pub fnc main(): int { return 42; }")
        .expect("unrelated fully supported scalar code must still compile");
    assert!(valid.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn ordered_lazy_map_filter_take_collect_is_typed_but_not_executable() {
    let mut plan = PipelinePlan::new(Family::Iterator, "int");
    plan.apply(filter("int", "bool", CallbackMode::ReadonlySync), pos()).unwrap();
    plan.apply(map("int", "string", CallbackMode::ReadonlySync, false), pos()).unwrap();
    plan.apply(Stage::Skip {count: 2}, pos()).unwrap();
    plan.apply(Stage::Take {count: 4}, pos()).unwrap();
    plan.apply(Stage::Collect {max_items: None}, pos()).unwrap();
    assert_eq!(plan.item_type, "string");
    assert_eq!(plan.bounded_items, Some(4));
    assert_eq!(plan.terminal_result.as_deref(),
        Some("Result<List<string>,CollectError>"));
    assert_eq!(plan.stages.len(), 5);
}

#[test]
fn async_terminal_returns_preserve_one_future_layer() {
    for family in [Family::AsyncIterator, Family::Stream] {
        let mut collect = PipelinePlan::new(family, "Event");
        collect.apply(Stage::Collect {max_items: Some(9)}, pos()).unwrap();
        assert_eq!(collect.terminal_result.as_deref(),
            Some("Future<Result<List<Event>,CollectError>>"));

        let mut fold = PipelinePlan::new(family, "int");
        fold.apply(Stage::Fold {input:"int".into(), accumulator:"int".into(),
            result:"int".into(), mode:CallbackMode::ReadonlySync}, pos()).unwrap();
        assert_eq!(fold.terminal_result.as_deref(), Some("Future<int>"));
    }
}

#[test]
fn unbounded_collect_and_skip_only_must_fail_closed() {
    let mut plan = PipelinePlan::new(Family::Stream, "int");
    plan.apply(Stage::Skip {count: 12}, pos()).unwrap();
    assert_eq!(plan.apply(Stage::Collect {max_items: None}, pos())
        .unwrap_err().code, "E_STREAM_UNBOUNDED_COLLECT");
    assert_eq!(plan.stages.len(), 1);
    plan.apply(Stage::Take {count: 3}, pos()).unwrap();
    plan.apply(Stage::Collect {max_items: None}, pos()).unwrap();
    assert_eq!(plan.bounded_items, Some(3));
}

#[test]
fn effectful_callbacks_and_unproven_consuming_ownership_rejected() {
    let mut plan = PipelinePlan::new(Family::Stream, "int");
    assert_eq!(plan.apply(map("int", "int", CallbackMode::Async, false), pos())
        .unwrap_err().code, "E_STREAM_CALLBACK_EFFECT");
    assert_eq!(plan.apply(map("int", "int", CallbackMode::ConsumingSync, false), pos())
        .unwrap_err().code, "E_ITERATOR_OWNERSHIP");
    assert_eq!(plan.apply(filter("int","bool",CallbackMode::Async),pos())
        .unwrap_err().code,"E_STREAM_CALLBACK_EFFECT");
    assert_eq!(plan.apply(filter("int","int",CallbackMode::ReadonlySync),pos())
        .unwrap_err().code,"E_STREAM_TYPE");
    assert!(plan.stages.is_empty());
    plan.apply(map("int", "Owned", CallbackMode::ConsumingSync, true), pos()).unwrap();
    assert_eq!(plan.item_type, "Owned");
}

#[test]
fn terminal_and_wrong_types_never_form_executable_chain() {
    let mut plan = PipelinePlan::new(Family::Iterator, "int");
    assert_eq!(plan.apply(map("bool","bool",CallbackMode::ReadonlySync,false),pos())
        .unwrap_err().code,"E_STREAM_TYPE");
    assert_eq!(plan.apply(Stage::Fold {input:"int".into(),accumulator:"int".into(),
        result:"bool".into(),mode:CallbackMode::ReadonlySync},pos())
        .unwrap_err().code,"E_STREAM_TYPE");
    plan.apply(Stage::Collect {max_items:Some(5)},pos()).unwrap();
    assert_eq!(plan.apply(Stage::Take {count:1},pos()).unwrap_err().code,
        "E_STREAM_TERMINAL");
}

#[test]
fn iterable_requires_iter_and_stage_depth_is_bounded() {
    let mut iterable = PipelinePlan::new(Family::Iterable, "int");
    assert_eq!(iterable.apply(Stage::Take {count:2},pos()).unwrap_err().code,
        "E_STREAM_TYPE");
    let mut iter = PipelinePlan::new(Family::Iterator, "int");
    for _ in 0..64 {
        iter.apply(Stage::Skip {count:1},pos()).unwrap();
    }
    assert_eq!(iter.apply(Stage::Skip {count:1},pos()).unwrap_err().code,
        "E_STREAM_QUOTA");
}

#[test]
fn take_multiple_times_uses_tightest_finite_bound() {
    let mut plan = PipelinePlan::new(Family::Iterator,"int");
    plan.apply(Stage::Take{count:100},pos()).unwrap();
    plan.apply(Stage::Take{count:4},pos()).unwrap();
    plan.apply(Stage::Take{count:50},pos()).unwrap();
    assert_eq!(plan.bounded_items,Some(4));
}
