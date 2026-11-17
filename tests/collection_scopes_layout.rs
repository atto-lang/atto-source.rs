//! Scoped Record field and layout-planning checks.
//! These are compiler-front-end checks, not guest memory/ABI execution.
use atto_compiler::{collection_scopes, collection_types::LiteralType as Ty,
    diag::{Diagnostic,Pos},layout_plan::{self,Identity},lexer,parser::Parser,ast::Program};

fn parsed(body:&str)->Result<Program,Diagnostic> {
    let source=format!("fnc main(): int {{\n{body}\nreturn 0;\n}}");
    Parser::new(lexer::lex(&source)?).parse_program()
}
fn accepted(body:&str) {
    let p=parsed(body).unwrap_or_else(|e|panic!("parse: {body}: {e}"));
    collection_scopes::analyze(&p).unwrap_or_else(|e|panic!("scope: {body}: {e}"));
}
fn rejects(body:&str,code:&str) {
    let p=parsed(body).unwrap_or_else(|e|panic!("parse: {body}: {e}"));
    let err=collection_scopes::analyze(&p).expect_err(body);
    assert_eq!(err.code,code,"{body}: {err}");
}
fn pos()->Pos {Pos {line:1,col:1}}

#[test]
fn scoped_dot_and_quoted_field_reads_accept_and_preserve_types() {
    accepted("let Record h = {\"content-type\": \"text/plain\", code: 200};\n\
        let string mime = h[\"content-type\"];\n\
        let int number = h.code;");
    accepted("let Record h = {nested: {\"x-y\": true}};\n\
        let bool inner = h.nested[\"x-y\"];");
    accepted("let Record h = {\"\": 42}; let int empty_name = h[\"\"];");
}
#[test]
fn closed_missing_and_bad_static_key_are_rejected() {
    rejects("let Record h = {code: 200}; let int x = h.missing;", "E_NO_FIELD");
    rejects("let Record h = {code: 200}; let int x = h[\"missing\"];", "E_NO_FIELD");
    rejects("let Record h = {code: 200}; let string key = \"code\"; let int x = h[key];",
        "E_RECORD_KEY");
    rejects("let Map<string,int> m = {\"code\" => 200}; let int x = m.code;", "E_TYPE");
    rejects("let Record h = {code: 200}; let bool x = h.code;", "E_TYPE");
}
#[test]
fn aliases_and_lexical_shadowing_use_nearest_shape() {
    accepted("let Record original = {code: 200}; let Record alias = original;\n\
        let int result = alias.code;");
    accepted("let Record h = {outer: 1}; block {\n\
        let Record h = {inner: true}; let bool value = h.inner;\n\
        } let int retained = h.outer;");
    rejects("let Record h = {outer: 1}; block {\n\
        let Record h = {inner: true}; let int bad = h.outer;\n\
        }", "E_NO_FIELD");
}
#[test]
fn branch_shape_join_never_proves_missing_field() {
    rejects("let Record h = {a: 1};\n\
        if true then h = {b: 2}; fi\n\
        let int x = h.a;", "E_FIELD_PRESENCE");
    accepted("let Record h = {a: 1};\n\
        if true then let int temporary = h.a; fi\n\
        let int x = h.a;");
    rejects("let Record h = {a: 1}; if true then\n\
        let Record h = {b: 2}; let int x = h.a; fi", "E_NO_FIELD");
}
#[test]
fn open_parameter_requires_field_proof_even_with_a_spelling() {
    let src="fnc inspect(Record h): int { return h.code; }";
    let p=Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(collection_scopes::analyze(&p).unwrap_err().code,"E_FIELD_PRESENCE");
}
#[test]
fn canonical_record_layout_is_order_independent() {
    let left=Ty::Record(vec![("z".into(),Ty::Bool),("a".into(),Ty::Int)]);
    let right=Ty::Record(vec![("a".into(),Ty::Int),("z".into(),Ty::Bool)]);
    let a=layout_plan::record(&left,pos()).unwrap();
    let b=layout_plan::record(&right,pos()).unwrap();
    assert_eq!(a,b);
    assert_eq!(a.identity,Identity::ClosedRecord);
    assert_eq!(a.fields.iter().map(|f|f.name.as_str()).collect::<Vec<_>>(),vec!["a","z"]);
    assert_eq!(a.fields.iter().map(|f|f.scalar_offset).collect::<Vec<_>>(),
        vec![Some(0),Some(8)]);
    assert_eq!(a.scalar_bytes,Some(16));
    assert_eq!(a.scalar_alignment,Some(8));
}
#[test]
fn struct_identity_is_nominal_and_declaration_order_is_preserved() {
    let fields=vec![("z".into(),Ty::Bool),("a".into(),Ty::Int)];
    let a=layout_plan::nominal_struct("First",&fields,pos()).unwrap();
    let b=layout_plan::nominal_struct("Second",&fields,pos()).unwrap();
    assert_ne!(a,b);
    assert_eq!(a.identity,Identity::NominalStruct("First".into()));
    assert_eq!(a.fields.iter().map(|f|f.name.as_str()).collect::<Vec<_>>(),vec!["z","a"]);
    assert_eq!(a.fields.iter().map(|f|f.scalar_offset).collect::<Vec<_>>(),
        vec![Some(0),Some(8)]);
    assert_eq!(a.scalar_bytes,Some(16));
}
#[test]
fn opaque_and_open_shapes_do_not_invent_guest_memory_offsets() {
    let opaque=layout_plan::record(&Ty::Record(vec![
        ("name".into(),Ty::String),("enabled".into(),Ty::Bool)
    ]),pos()).unwrap();
    assert_eq!(opaque.scalar_bytes,None);
    assert!(opaque.fields.iter().all(|f|f.scalar_offset.is_none()));
    assert_eq!(layout_plan::record(&Ty::OpenRecord,pos()).unwrap_err().code,"E_LAYOUT_OPEN");
    assert_eq!(layout_plan::record(&Ty::Map(Box::new(Ty::String),Box::new(Ty::Int)),pos())
        .unwrap_err().code,"E_TYPE");
}
#[test]
fn structural_slot_collisions_are_compile_errors() {
    let dup=layout_plan::record(&Ty::Record(vec![
        ("x".into(),Ty::Int),("x".into(),Ty::Bool)
    ]),pos()).unwrap_err();
    assert_eq!(dup.code,"E_DUPLICATE");
}
