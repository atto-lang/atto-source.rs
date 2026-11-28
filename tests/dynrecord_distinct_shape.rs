use atto_compiler::{compile,lexer,parser::Parser,collection_types::{self,LiteralType as T},record_struct_layout::{LayoutInterner,ShapeKind}};
#[test]
fn shape_kind_separates_identical_fields_without_losing_static_slots() {
    let fields=vec![("x".into(),T::Int)];
    let mut layouts=LayoutInterner::new();
    let a=layouts.closed_record(&fields).unwrap();
    let b=layouts.closed_dynrecord(&fields).unwrap();
    assert_ne!(a.shape_id,b.shape_id);
    assert_eq!(a.source_to_slot,b.source_to_slot);
    assert_eq!(layouts.shape(b.shape_id).unwrap().kind,ShapeKind::DynRecord);
    assert_eq!(layouts.closed_dynrecord(&fields).unwrap(),b);
    assert!(compile(r#"pub fnc main():int{val r={x:40};val d=("x"=2);return r.x+d.x;}"#).is_ok());
}
#[test]
fn explicit_product_kinds_do_not_coerce_and_empty_parens_are_contextual() {
    for source in [r#"pub fnc main():int{val Record r=("x"=1);return 0;}"#,r#"pub fnc main():int{val DynRecord r={x:1};return 0;}"#] {
        assert_eq!(compile(source).unwrap_err().code,"E_TYPE");
    }
    let p=Parser::new(lexer::lex("pub fnc main():int{const DynRecord d=(); const Record r=();return 0;}").unwrap()).parse_program().unwrap();
    let t=collection_types::analyze(&p).unwrap();
    assert_eq!(t[0].value.ty,T::DynRecord(vec![]));
    assert_eq!(t[1].value.ty,T::Record(vec![]));
}
