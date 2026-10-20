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
fn unresolved_calls_are_rejected_for_cfg_export() {
    let parsed = frontend::parse("fnc f(): int { return g(); } fnc g(): int { return 1; }").unwrap();
    let module = mir::lower(parsed).unwrap();
    assert!(flow::export_scalar_flow(&module, 5).is_err());
}
