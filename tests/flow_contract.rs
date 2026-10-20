use oreslang_source_rs::{flow, frontend, mir};

#[test]
fn pure_scalar_hir_emits_analyzer_cfg_envelope() {
    let source = include_str!("fixtures/flow-scalar.ores");
    let parsed = frontend::parse(source).unwrap();
    let module = mir::lower(parsed).unwrap();
    let actual = flow::export_scalar_flow(&module, 6).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow-scalar.json")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn resolved_forward_scalar_calls_match_analyzer_golden() {
    let source = include_str!("fixtures/flow-forward.ores");
    let parsed = frontend::parse(source).unwrap();
    let module = mir::lower(parsed).unwrap();
    let actual = flow::export_scalar_flow(&module, 6).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow-forward.json")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn unknown_call_is_rejected_by_semantic_mir_pass() {
    let parsed = frontend::parse("fnc f(): int { return missing(); }").unwrap();
    assert!(mir::lower(parsed).is_err());
}
