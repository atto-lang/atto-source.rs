//! Readonly structural ABI witness tests. The witness is NOT Wasm codegen.
use atto_compiler::{compile, diag::Pos, readonly_shape as shape, structural_syntax};
use shape::{LoanEvidence, Origin, ReadPath, ReadType, RequestedField, Scalar, SourceField, SourceShape, Literal};

fn pos() -> Pos { Pos { line: 1, col: 1 } }
fn scalar(t: Scalar) -> ReadType { ReadType::Scalar(t) }
fn req(name: &str, ty: ReadType) -> RequestedField {
    RequestedField { name: name.into(), ty }
}
fn field(name: &str, ty: ReadType, path: ReadPath) -> SourceField {
    SourceField {
        name: name.into(),
        ty,
        public: true,
        protected_state: false,
        guaranteed_present: true,
        path,
    }
}
fn source(origin: Origin, fields: Vec<SourceField>) -> SourceShape {
    SourceShape {
        origin, fields,
        loan: LoanEvidence {
            owner_slot: 7, live_for_call: true, stable_against_writes: true,
        },
    }
}
fn error_code(s: &SourceShape, want: &[RequestedField]) -> &'static str {
    shape::plan_readonly(s, want, pos()).unwrap_err().code
}

#[test]
fn cross_nominal_classes_use_actual_offsets_not_formal_layout() {
    // Formal Point can have x at 0, y at 8. The source class has extras,
    // and its readable x/y are at 16/24. A nominal pointer cast is wrong.
    let s = source(Origin::Class, vec![
        field("tag", scalar(Scalar::Bool), ReadPath::Offset(0)),
        field("padding", scalar(Scalar::Float), ReadPath::Offset(8)),
        field("x", scalar(Scalar::Int), ReadPath::Offset(16)),
        field("y", scalar(Scalar::Int), ReadPath::Offset(24)),
    ]);
    let p = shape::plan_readonly(&s, &[
        req("x", scalar(Scalar::Int)), req("y", scalar(Scalar::Int))
    ], pos()).unwrap();
    assert_eq!(p.owner_slot, 7);
    assert_eq!(p.origin, Origin::Class);
    assert_eq!(p.fields[0].path, ReadPath::Offset(16));
    assert_eq!(p.fields[1].path, ReadPath::Offset(24));
    assert_eq!(p.fields.len(), 2);
}

#[test]
fn records_and_structs_can_satisfy_open_row_formals_without_layout_erasure() {
    for origin in [Origin::Record, Origin::Struct] {
        let s = source(origin, vec![
            field("foo", scalar(Scalar::Str), ReadPath::Offset(32)),
            field("extra", scalar(Scalar::Int), ReadPath::Offset(8)),
        ]);
        let p = shape::plan_readonly(&s,
            &[req("foo", scalar(Scalar::Str))], pos()).unwrap();
        assert_eq!(p.origin, origin);
        assert_eq!(p.fields[0].path, ReadPath::Offset(32));
    }
}

#[test]
fn singleton_means_guaranteed_type_not_a_mutable_default_value() {
    let want = [req("foo", ReadType::Singleton(Literal::Str("dog".into())))];
    let mutable_class = source(Origin::Class, vec![
        field("foo", scalar(Scalar::Str), ReadPath::Offset(0))
    ]);
    // Even if the class constructor defaults foo to dog, string is wider.
    assert_eq!(error_code(&mutable_class, &want), "E_TYPE");
    let immutable_record = source(Origin::Record, vec![
        field("foo", ReadType::Singleton(Literal::Str("dog".into())), ReadPath::Offset(0))
    ]);
    assert!(shape::plan_readonly(&immutable_record, &want, pos()).is_ok());
    assert!(shape::plan_readonly(&immutable_record,
        &[req("foo", scalar(Scalar::Str))], pos()).is_ok());
    assert_eq!(error_code(&immutable_record,
        &[req("foo", ReadType::Singleton(Literal::Str("cat".into())))]), "E_TYPE");
}

#[test]
fn literal_and_union_read_covariance_is_total_over_source_variants() {
    let string_or_int = ReadType::Union(vec![scalar(Scalar::Str), scalar(Scalar::Int)]);
    let string_source = source(Origin::Class,
        vec![field("foo", scalar(Scalar::Str), ReadPath::Offset(0))]);
    assert!(shape::plan_readonly(&string_source, &[req("foo", string_or_int.clone())], pos()).is_ok());

    let two_type_source = source(Origin::Record, vec![
        field("foo", string_or_int.clone(), ReadPath::Offset(0))
    ]);
    assert!(shape::plan_readonly(&two_type_source,
        &[req("foo", string_or_int.clone())], pos()).is_ok());
    // Accepting any one alternative would unsoundly admit the int branch.
    assert_eq!(error_code(&two_type_source,
        &[req("foo", scalar(Scalar::Str))]), "E_TYPE");
    let wider = ReadType::Union(vec![scalar(Scalar::Str), scalar(Scalar::Int), scalar(Scalar::Bool)]);
    assert!(shape::plan_readonly(&two_type_source,
        &[req("foo", wider)], pos()).is_ok());

    let literal_source = source(Origin::Record, vec![
        field("foo", ReadType::Singleton(Literal::Int(-1)), ReadPath::Offset(0))
    ]);
    assert!(shape::plan_readonly(&literal_source,
        &[req("foo", scalar(Scalar::Int))], pos()).is_ok());
    assert_eq!(error_code(&literal_source,
        &[req("foo", ReadType::Singleton(Literal::Int(1)))]), "E_TYPE");
    let bool_source = source(Origin::Record, vec![
        field("foo", ReadType::Singleton(Literal::Bool(true)), ReadPath::Offset(0))
    ]);
    assert!(shape::plan_readonly(&bool_source,
        &[req("foo", scalar(Scalar::Bool))], pos()).is_ok());
}

#[test]
fn privacy_sealed_actor_state_and_effectful_getters_fail_closed() {
    let wanted = [req("foo", scalar(Scalar::Int))];
    let mut private = source(Origin::Class, vec![
        field("foo", scalar(Scalar::Int), ReadPath::Offset(0))
    ]);
    private.fields[0].public = false;
    assert_eq!(error_code(&private, &wanted), "E_PRIVATE");
    private.fields[0].public = true;
    private.fields[0].protected_state = true;
    assert_eq!(error_code(&private, &wanted), "E_STATE_BORROW");
    private.fields[0].protected_state = false;
    private.fields[0].path = ReadPath::Getter;
    assert_eq!(error_code(&private, &wanted), "E_UNSUPPORTED");
    assert_eq!(error_code(&private, &[req("missing", scalar(Scalar::Int))]), "E_TYPE");
}

#[test]
fn map_requires_a_real_presence_proof_and_stable_value_snapshot() {
    let wanted = [req("foo", scalar(Scalar::Str))];
    let mut map = source(Origin::Map, vec![
        field("foo", scalar(Scalar::Str), ReadPath::UnprovenKey("foo".into()))
    ]);
    assert_eq!(error_code(&map, &wanted), "E_TYPE");
    map.fields[0].path = ReadPath::GuaranteedKey { key: "foo".into(), proof: 17 };
    assert!(shape::plan_readonly(&map, &wanted, pos()).is_ok());
    map.fields[0].path = ReadPath::SnapshotKey { key: "foo".into(), snapshot: 19 };
    let projected = shape::plan_readonly(&map, &wanted, pos()).unwrap();
    assert_eq!(projected.fields[0].path, map.fields[0].path);
    map.fields[0].path = ReadPath::SnapshotKey { key: "foo".into(), snapshot: 0 };
    assert_eq!(error_code(&map, &wanted), "E_TYPE");
    map.fields[0].path = ReadPath::GuaranteedKey { key: "other".into(), proof: 3 };
    assert_eq!(error_code(&map, &wanted), "E_TYPE");
    map.fields[0].path = ReadPath::GuaranteedKey { key: "foo".into(), proof: 3 };
    map.fields[0].guaranteed_present = false;
    assert_eq!(error_code(&map, &wanted), "E_TYPE");
}

#[test]
fn loans_and_descriptor_integrity_must_be_proven_before_emission() {
    let wanted = [req("foo", scalar(Scalar::Int))];
    let mut s = source(Origin::Class, vec![
        field("foo", scalar(Scalar::Int), ReadPath::Offset(0))
    ]);
    s.loan.live_for_call = false;
    assert_eq!(error_code(&s, &wanted), "E_BORROW");
    s.loan.live_for_call = true;
    s.loan.stable_against_writes = false;
    assert_eq!(error_code(&s, &wanted), "E_BORROW");
    s.loan.stable_against_writes = true;
    s.fields.push(field("foo", scalar(Scalar::Int), ReadPath::Offset(8)));
    assert_eq!(error_code(&s, &wanted), "E_DUPLICATE");
    s.fields[1].name = "bar".into();
    s.fields[1].path = ReadPath::Offset(0);
    assert_eq!(error_code(&s, &wanted), "E_TYPE");
    s.fields[1].path = ReadPath::Offset(8);
    assert_eq!(error_code(&s, &[wanted[0].clone(), wanted[0].clone()]), "E_DUPLICATE");
    let many = (0..33).map(|i|
        req(&format!("f{i}"), scalar(Scalar::Int))).collect::<Vec<_>>();
    assert_eq!(error_code(&s, &many), "E_SOURCE_LIMIT");
    let union = ReadType::Union(vec![scalar(Scalar::Int); 17]);
    assert_eq!(error_code(&s, &[req("foo", union)]), "E_SOURCE_LIMIT");
}

#[test]
fn named_argument_plan_evaluates_once_then_permutes_by_formal_slot() {
    let formals = structural_syntax::parse_formals("(item: ~Point, scale: int, verbose: bool)").unwrap();
    let args = structural_syntax::parse_arguments(
        "(~p, verbose: expensive_flag(), scale: expensive_scale())"
    ).unwrap();
    let slots = structural_syntax::bind_checked_structural(&formals, &args).unwrap();
    assert_eq!(slots, [0, 2, 1]);
    let plan = shape::plan_call_evaluation(&slots, 3, pos()).unwrap();
    assert_eq!(plan.evaluations.iter().map(|s| s.source_argument).collect::<Vec<_>>(),
        [0, 1, 2]);
    assert_eq!(plan.evaluations.iter().map(|s| s.parameter).collect::<Vec<_>>(),
        [0, 2, 1]);
    assert_eq!(plan.parameter_temps, [0, 2, 1]);
    for slots in [&[0, 0, 2][..], &[0, 4, 2][..]] {
        assert!(shape::plan_call_evaluation(slots, 3, pos()).is_err());
    }
    assert_eq!(shape::plan_call_evaluation(&[0, 1], 3, pos()).unwrap_err().code, "E_ARITY");
}

#[test]
fn witness_tests_never_remove_the_production_codegen_gate() {
    let source = "define class Point as pub int x=10; end \
         pub fnc read(p: ~Point): int { return p.x; } \
         pub fnc main(): int { return read(new Point()); }";
    let err = compile(source).expect_err("a computed receiver has no stable loan and cannot manufacture a Wasm call");
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("scalar projection"));
}
