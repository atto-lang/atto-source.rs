use oreslang_source_rs::{effects, frontend, mir};
#[test]
fn emits_contract_fixture_without_schema_drift() {
    let src = include_str!("fixtures/forward.ores");
    let parsed = frontend::parse(src).expect("fixture syntax");
    let module = mir::lower(parsed).expect("fixture semantic checks");
    let emitted = effects::export_effect_ir(&module);
    let expected: serde_json::Value = serde_json::from_str(
        include_str!("fixtures/forward.effects.json")
    ).unwrap();
    assert_eq!(emitted, expected, "shared effect IR v1 contract drift");
}
