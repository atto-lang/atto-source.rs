//! HOST-ONLY static shape/layout-plan tests; this is not guest ABI evidence.
use atto_compiler::{
    ast::Program,
    collection_types::{self, LiteralType as T},
    diag::Diagnostic,
    lexer,
    parser::Parser,
    record_struct_layout::{
        decide_access, Error, FieldDecision, FieldType, LayoutInterner, ShapeKind,
        MAX_LAYOUT_NAME_BYTES, MAX_STATIC_FIELDS,
    },
};

fn named(name: &str, ty: T) -> FieldType {
    (name.to_string(), ty)
}

fn bindings(source: &str) -> Result<Vec<collection_types::TypedBinding>, Diagnostic> {
    let full_source = format!("fnc main(): int {{\n{source}\nreturn 0;\n}}");
    let program: Program = Parser::new(lexer::lex(&full_source)?).parse_program()?;
    collection_types::analyze(&program)
}

#[test]
fn closed_record_type_identity_ignores_source_field_order() {
    let values = bindings(
        "let Record a = {z: true, a: 42};\n\
         let Record b = {a: 42, z: true};",
    )
    .unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].value.ty, values[1].value.ty);
    assert_eq!(
        values[0].value.ty,
        T::Record(vec![named("a", T::Int), named("z", T::Bool)])
    );
    // Source-order child evaluation is preserved; only layout slots differ.
    assert_eq!(values[0].value.children[0].ty, T::Bool);
    assert_eq!(values[0].value.children[1].ty, T::Int);
    assert_eq!(values[0].value.source_to_record_slot, vec![1, 0]);
    assert_eq!(values[1].value.source_to_record_slot, vec![0, 1]);
}

#[test]
fn equivalent_record_permutations_intern_one_static_shape() {
    let mut layouts = LayoutInterner::new();
    let a = layouts
        .closed_record(&[named("z", T::Bool), named("a", T::Int)])
        .unwrap();
    let b = layouts
        .closed_record(&[named("a", T::Int), named("z", T::Bool)])
        .unwrap();
    assert_eq!(a.shape_id, b.shape_id);
    assert_eq!(a.source_to_slot, vec![1, 0]);
    assert_eq!(b.source_to_slot, vec![0, 1]);
    assert_eq!(layouts.len(), 1);
    let shape = layouts.shape(a.shape_id).unwrap();
    assert_eq!(shape.kind, ShapeKind::Record);
    assert_eq!(shape.fields, vec![named("a", T::Int), named("z", T::Bool)]);
    assert_eq!(shape.slot("a"), Some(0));
    assert_eq!(shape.slot("z"), Some(1));
    assert_eq!(shape.slot("missing"), None);
}

#[test]
fn quoted_utf8_record_fields_are_static_but_not_struct_identifiers() {
    let values =
        bindings(r#"let Record headers = {"content-type": "text/plain", status_code: 200};"#)
            .unwrap();
    assert_eq!(
        values[0].value.ty,
        T::Record(vec![
            named("content-type", T::String),
            named("status_code", T::Int),
        ])
    );
    let mut registry = LayoutInterner::new();
    let p = registry
        .closed_record(&[
            named("status_code", T::Int),
            named("content-type", T::String),
            named("über field", T::Bool),
        ])
        .unwrap();
    assert_eq!(p.source_to_slot, vec![1, 0, 2]);
    assert_eq!(
        registry.shape(p.shape_id).unwrap().slot("content-type"),
        Some(0)
    );
    assert_eq!(
        registry.shape(p.shape_id).unwrap().slot("über field"),
        Some(2)
    );
    assert_eq!(
        registry.nominal_struct("app::Header", &[named("content-type", T::String),]),
        Err(Error::InvalidStructField)
    );
}

#[test]
fn map_and_open_record_do_not_pretend_to_be_fixed_slot_records() {
    let ty = T::Record(vec![named("a", T::Int), named("b", T::Bool)]);
    assert_eq!(decide_access(&ty, "a"), FieldDecision::StaticSlot(0));
    assert_eq!(decide_access(&ty, "b"), FieldDecision::StaticSlot(1));
    assert_eq!(decide_access(&ty, "absent"), FieldDecision::Missing);
    assert_eq!(
        decide_access(&T::OpenRecord, "a"),
        FieldDecision::CheckOpenShape
    );
    assert_eq!(
        decide_access(&T::Map(Box::new(T::String), Box::new(T::Int)), "a"),
        FieldDecision::RuntimeMapKey
    );
    assert_eq!(
        decide_access(&T::Int, "a"),
        FieldDecision::NotFieldContainer
    );
}

#[test]
fn nominal_structs_are_distinct_despite_identical_field_types() {
    let fields = vec![named("z", T::Bool), named("a", T::Int)];
    let mut layouts = LayoutInterner::new();
    let first = layouts.nominal_struct("pkg::First", &fields).unwrap();
    let again = layouts.nominal_struct("pkg::First", &fields).unwrap();
    let second = layouts.nominal_struct("pkg::Second", &fields).unwrap();
    let record = layouts.closed_record(&fields).unwrap();
    assert_eq!(first.shape_id, again.shape_id);
    assert_ne!(first.shape_id, second.shape_id);
    assert_ne!(first.shape_id, record.shape_id);
    assert_eq!(layouts.shape(first.shape_id).unwrap().slot("z"), Some(0));
    assert_eq!(layouts.shape(first.shape_id).unwrap().slot("a"), Some(1));
    assert_eq!(first.source_to_slot, vec![0, 1]);
    assert_eq!(layouts.shape(record.shape_id).unwrap().slot("a"), Some(0));
    assert_eq!(layouts.len(), 3);
}

#[test]
fn a_nominal_struct_cannot_change_schema_under_the_same_identity() {
    let mut layouts = LayoutInterner::new();
    layouts
        .nominal_struct("pkg::Header", &[named("status", T::Int)])
        .unwrap();
    assert_eq!(
        layouts.nominal_struct("pkg::Header", &[named("status", T::Bool),]),
        Err(Error::ChangedNominalDefinition)
    );
    assert_eq!(layouts.len(), 1);
}

#[test]
fn duplicates_and_unresolved_types_are_rejected_not_guessed() {
    let mut layouts = LayoutInterner::new();
    assert_eq!(
        layouts.closed_record(&[named("a", T::Int), named("a", T::Bool),]),
        Err(Error::DuplicateField)
    );
    assert_eq!(
        layouts.closed_record(&[named("a", T::Bottom),]),
        Err(Error::UnresolvedType)
    );
    assert_eq!(
        layouts.closed_record(&[named("rows", T::Array(Box::new(T::OpenRecord))),]),
        Err(Error::UnresolvedType)
    );
    assert_eq!(
        layouts.closed_record(&[named("tuple", T::AnyTuple),]),
        Err(Error::UnresolvedType)
    );
    assert!(layouts.is_empty());
}

#[test]
fn no_unicode_normalization_or_implicit_case_folding() {
    let mut layouts = LayoutInterner::new();
    let a = layouts.closed_record(&[named("e\u{301}", T::Int)]).unwrap();
    let b = layouts.closed_record(&[named("é", T::Int)]).unwrap();
    let c = layouts.closed_record(&[named("A", T::Int)]).unwrap();
    let d = layouts.closed_record(&[named("a", T::Int)]).unwrap();
    assert_ne!(a.shape_id, b.shape_id);
    assert_ne!(c.shape_id, d.shape_id);
}

#[test]
fn duplicate_bare_and_quoted_labels_fail_at_parser_boundary() {
    assert_eq!(
        bindings(r#"let Record r = {name: 1, "name": 2};"#)
            .unwrap_err()
            .code,
        "E_DUPLICATE"
    );
}

#[test]
fn no_excessive_static_shape_or_field_allocation() {
    let mut registry = LayoutInterner::with_shape_limit(1);
    let a = registry.closed_record(&[named("id", T::Int)]).unwrap();
    let again = registry.closed_record(&[named("id", T::Int)]).unwrap();
    assert_eq!(a.shape_id, again.shape_id);
    assert_eq!(
        registry.closed_record(&[named("other", T::Int)]),
        Err(Error::TooManyShapes)
    );

    let many: Vec<FieldType> = (0..MAX_STATIC_FIELDS + 1)
        .map(|i| named(&format!("f{i}"), T::Int))
        .collect();
    assert_eq!(
        LayoutInterner::new().closed_record(&many),
        Err(Error::TooManyFields)
    );

    let huge = "x".repeat(MAX_LAYOUT_NAME_BYTES + 1);
    assert_eq!(
        LayoutInterner::new().closed_record(&[named(&huge, T::Int)]),
        Err(Error::TooManyNameBytes)
    );
}

#[test]
fn fixed_slot_plan_is_not_guest_codegen_authorization() {
    // The current Rust/browser compiler intentionally rejects allocation
    // of records/maps/structs. A host-only shape plan is no permission to
    // emit an object with fabricated offsets or bypass ownership.
    let code = "fnc main(): int {let Record r = {name: \"Atto\", age: 3}; return 0;}";
    let err = atto_compiler::compile(code).expect_err("Record guest lowering not yet supported");
    assert_eq!(err.code, "E_UNSUPPORTED");
}
