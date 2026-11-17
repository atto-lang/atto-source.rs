//! Static collection flow tests. These must not be interpreted as runtime
//! ownership, allocator, borrower, or WebAssembly execution evidence.
use atto_compiler::{
    ast::Program,
    collection_types::{self, LiteralType as T},
    diag::Diagnostic, lexer, parser::Parser,
};

fn analyze(body: &str) -> Result<Vec<collection_types::TypedBinding>, Diagnostic> {
    let source = format!("fnc main(): int {{\n{body}\nreturn 0;\n}}");
    let program: Program = Parser::new(lexer::lex(&source)?).parse_program()?;
    collection_types::analyze(&program)
}

#[test]
fn scalar_bindings_flow_into_structural_records() {
    let typed = analyze(
        "let n = 7;\nlet value = {job: n, valid: true};\nlet output = {score: value.job};"
    ).unwrap();
    assert_eq!(typed.len(), 2);
    assert_eq!(typed[0].value.ty, T::Record(vec![
        ("job".into(),T::Int),("valid".into(),T::Bool),
    ]));
    assert_eq!(typed[1].value.ty,
        T::Record(vec![("score".into(),T::Int)]));
}

#[test]
fn record_member_projection_has_checked_scalar_type() {
    let typed = analyze("let data = {age: 3};\nlet age = data.age;").unwrap();
    assert_eq!(typed.len(), 2);
    assert_eq!(typed[1].value.ty,T::Int);
    assert_eq!(typed[1].name,"age");
}

#[test]
fn nested_record_heterogeneity_preserves_shapes_with_local_scalar_references() {
    let typed = analyze(
        "let age = 29;\nlet array<record> rows = [{age: age}, {name: \"Atto\"}];"
    ).unwrap();
    assert_eq!(typed.len(),1);
    assert_eq!(typed[0].value.ty,T::Array(Box::new(T::OpenRecord)));
    assert_eq!(typed[0].value.children[0].ty,
        T::Record(vec![("age".into(),T::Int)]));
    assert_eq!(typed[0].value.children[1].ty,
        T::Record(vec![("name".into(),T::String)]));
}

#[test]
fn scalar_arithmetic_is_checked_without_converting_to_any() {
    let typed = analyze("let age=20;\nlet user={year: age+5};").unwrap();
    assert_eq!(typed[0].value.ty,
        T::Record(vec![("year".into(),T::Int)]));
    assert_eq!(analyze("let year=true;\nlet r={year:year+1};").unwrap_err().code,
        "E_TYPE");
}

#[test]
fn unknown_and_missing_record_fields_are_diagnosed() {
    assert_eq!(
        analyze("let rec={a:1};\nlet field=rec.b;").unwrap_err().code,
        "E_NO_FIELD");
    assert_eq!(
        analyze("let Record rec={a:1};\nlet field=rec.a;").unwrap_err().code,
        "E_FIELD_PRESENCE");
    assert_eq!(
        analyze("let rec={a:missing};").unwrap_err().code,
        "E_UNBOUND");
}

#[test]
fn implicit_aliases_of_owned_records_lists_and_fields_reject() {
    for body in [
        "let rec={age:1};\nlet alias=rec;",
        "let list=[1,2];\nlet alias=list;",
        "let rec={ages:[1]};\nlet alias=rec.ages;",
        "let rec={age:1};\nlet other={same:rec};",
        "let original=[1,2];\nlet group=[original];",
    ] {
        assert_eq!(analyze(body).unwrap_err().code,"E_OWNERSHIP","{body}");
    }
}

#[test]
fn changes_to_aggregate_locals_must_be_ownership_checked() {
    let err = analyze("let rec={age:1};\nrec={age:2};").unwrap_err();
    assert_eq!(err.code,"E_OWNERSHIP");
}

#[test]
fn lexical_scope_does_not_leak_and_scalar_dependencies_are_sequential() {
    let typed = analyze(
        "let age=1;\nblock { let age=2; let inner={value:age}; }\nlet outer={value:age};"
    ).unwrap();
    assert_eq!(typed.len(),2);
    assert_eq!(typed[0].value.ty,T::Record(vec![("value".into(),T::Int)]));
    assert_eq!(typed[1].value.ty,T::Record(vec![("value".into(),T::Int)]));
}

#[test]
fn records_preserve_source_field_evaluation_order_and_canonical_type_order() {
    let typed = analyze("let rec={z:true, a:3};").unwrap();
    assert_eq!(typed[0].value.ty,T::Record(vec![
        ("a".into(),T::Int),("z".into(),T::Bool),
    ]));
    assert_eq!(typed[0].value.children[0].ty,T::Bool);
    assert_eq!(typed[0].value.children[1].ty,T::Int);
}

#[test]
fn signed_minimum_is_not_a_positive_literal() {
    assert_eq!(
        analyze("let rec={value:9223372036854775808};").unwrap_err().code,
        "E_INTEGER_RANGE");
    let typed = analyze("let rec={value:-9223372036854775808};").unwrap();
    assert_eq!(typed[0].value.ty,T::Record(vec![("value".into(),T::Int)]));
}
