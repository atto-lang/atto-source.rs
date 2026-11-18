//! Executed by \`cargo test --test inline_collection_types\` when CI admits a runner.
//! These are type-front-end tests, NOT guest-Wasm execution or allocation proof.
use atto_compiler::{
    ast::Program, collection_types::{self, LiteralType as T},
    compile, diag::Diagnostic, lexer, parser::Parser,
};

fn parsed(body: &str) -> Result<Program, Diagnostic> {
    let source = format!("fnc main(): int {{\n{body}\nreturn 0;\n}}");
    Parser::new(lexer::lex(&source)?).parse_program()
}
fn typed(body: &str) -> Result<Vec<collection_types::TypedBinding>, Diagnostic> {
    collection_types::analyze(&parsed(body)?)
}
fn one(body: &str) -> collection_types::TypedBinding {
    let mut items = typed(body).unwrap_or_else(|e| panic!("{body}: {e}"));
    assert_eq!(items.len(), 1, "{body}");
    items.remove(0)
}

#[test]
fn heterogeneous_arrays_preserve_nested_record_shapes() {
    let upper = one(
        "let Array<Record> rows = [{job:1}, {car:\"str\"}, {x:true}];"
    );
    let lower = one(
        "let array<record> rows = [{job:1}, {car:\"str\"}, {x:true}];"
    );
    assert_eq!(upper, lower);
    assert_eq!(upper.declared, Some(T::Array(Box::new(T::OpenRecord))));
    assert_eq!(upper.value.ty, T::Array(Box::new(T::OpenRecord)));
    assert_eq!(upper.value.children.iter().map(|n| n.ty.clone()).collect::<Vec<_>>(),
       vec![
           T::Record(vec![("job".into(),T::Int)]),
           T::Record(vec![("car".into(),T::String)]),
           T::Record(vec![("x".into(),T::Bool)]),
       ]);
}

#[test]
fn record_fields_retain_nested_empty_list_bottom() {
    let x = one("let Record foo = {bar: []};");
    assert_eq!(x.value.ty,
        T::Record(vec![("bar".into(), T::List(Box::new(T::Bottom)))]));
    assert_eq!(x.value.children.len(), 1);
    let y = one("let record foo = {bar: []};");
    assert_eq!(x,y);
}

#[test]
fn map_keys_and_values_are_inferred_independently() {
    let x = one("let Map foo = {\n \"1\" => 1\n \"2\" => 3\n};");
    assert_eq!(x.value.ty, T::Map(Box::new(T::String),Box::new(T::Int)));
    let y = one("let map<string,int> foo = {\"1\" => 1, \"2\" => 3};");
    assert_eq!(x.value.ty,y.value.ty);
    // Source order: each key node is immediately followed by its value node.
    assert_eq!(y.value.children.iter().map(|n|n.ty.clone()).collect::<Vec<_>>(),
        vec![T::String,T::Int,T::String,T::Int]);
}

#[test]
fn empty_collections_need_context_except_bare_list() {
    let list = one("let List foo = [];");
    assert_eq!(list.value.ty,T::List(Box::new(T::Bottom)));
    let array = one("let array<int> foo = [];");
    assert_eq!(array.value.ty,T::Array(Box::new(T::Int)));
    let record = one("let Record foo = {};");
    assert_eq!(record.value.ty,T::Record(vec![]));
    let map = one("let Map<int,string> foo = {};");
    assert_eq!(map.value.ty,T::Map(Box::new(T::Int),Box::new(T::String)));
    for s in ["let foo = [];", "let foo = {};", "let Map foo = {};"] {
        assert_eq!(typed(s).unwrap_err().code,"E_INFER","{s}");
    }
}

#[test]
fn tuple_is_arity_and_position_sensitive() {
    assert_eq!(one("let Tuple foo = ();").value.ty,T::Tuple(vec![]));
    assert_eq!(one("let tuple foo = (1,);").value.ty,T::Tuple(vec![T::Int]));
    assert_eq!(one("let Tuple foo = (1,\"a\",true);").value.ty,
        T::Tuple(vec![T::Int,T::String,T::Bool]));
    assert_eq!(one("let Tuple<int,bool> foo = (1,true);").value.ty,
        T::Tuple(vec![T::Int,T::Bool]));
    assert_eq!(typed("let Tuple<int,bool> foo = (1,);").unwrap_err().code,"E_TYPE_ARITY");
    assert_eq!(typed("let Tuple<int,bool> foo = (1,2);").unwrap_err().code,"E_TYPE");
}

#[test]
fn wrong_collection_kinds_and_item_types_reject_before_wasm() {
    for s in [
        "let Array<int> foo = [1,\"bad\"];",
        "let List<string> foo = [\"a\",3];",
        "let Map<int,string> foo = {\"wrong\" => \"value\"};",
        "let Record foo = {\"a\" => 1};",
        "let Map foo = {a: 1};",
        "let Map foo = {1 => 1, \"x\" => 2};",
        "let List foo = [1, true];",
    ] {
        assert_eq!(typed(s).unwrap_err().code,"E_TYPE","{s}");
    }
    assert_eq!(typed("let Map foo = {{x:1} => 2};").unwrap_err().code,"E_MAP_KEY");
    assert_eq!(typed("let Array<Record> foo = [{x: true}, [42]];").unwrap_err().code,"E_TYPE");
}

#[test]
fn static_typing_success_is_not_wasm_execution() {
    for s in [
        "let Record foo = {bar: []};",
        "let Map foo = {\"1\" => 1};",
        "let List foo = [];",
        "let Array<Record> foo = [{job: 1}];",
        "let Tuple foo = ();",
        "let array<record> foo = [{job: 1}, {car: \"str\"}];",
    ] {
        typed(s).unwrap();
        let source = format!("fnc main(): int {{\n{s}\nreturn 0;\n}}");
        let err = compile(&source).expect_err("guest lowering is deliberately blocked");
        assert_eq!(err.code,"E_UNSUPPORTED","{s}: {err}");
    }
}

#[test]
fn nested_generic_types_have_distinct_shapes() {
    assert_eq!(one("let array<list<int>> foo = [[1,2],[3]];").value.ty,
        T::Array(Box::new(T::List(Box::new(T::Int)))));
    assert_eq!(one("let map<string,tuple<int,bool>> foo = {\"x\" => (1,true)};").value.ty,
        T::Map(Box::new(T::String), Box::new(T::Tuple(vec![T::Int,T::Bool]))));
}


#[test]
fn record_shape_is_order_independent_but_initializer_order_is_retained() {
    let left = one("let Record x = {\"content-type\": 1, status_code: true};");
    let right = one("let record x = {status_code: true, \"content-type\": 1};");
    assert_eq!(left.value.ty, right.value.ty,
        "structural record type identity must not depend on declaration order");
    assert_eq!(left.value.children.iter().map(|v| &v.ty).collect::<Vec<_>>(),
        vec![&T::Int, &T::Bool], "initializer order remains source order");
    assert_eq!(right.value.children.iter().map(|v| &v.ty).collect::<Vec<_>>(),
        vec![&T::Bool, &T::Int], "reordered initializer order is preserved");
    assert_eq!(
        collection_types::static_record_field_type(&left.value.ty, "content-type", left.value.pos).unwrap(),
        T::Int
    );
    assert_eq!(
        collection_types::static_record_field_type(&right.value.ty, "status_code", right.value.pos).unwrap(),
        T::Bool
    );
}

#[test]
fn closed_record_missing_fields_and_open_record_unguarded_reads_reject() {
    let row = one("let Record x = {\"\": 1, \"x-y\": true};");
    assert_eq!(
        collection_types::static_record_field_type(&row.value.ty, "", row.value.pos).unwrap(),
        T::Int
    );
    let missing = collection_types::static_record_field_type(&row.value.ty, "absent", row.value.pos).unwrap_err();
    assert_eq!(missing.code, "E_NO_FIELD");
    let open = collection_types::static_record_field_type(&T::OpenRecord, "x-y", row.value.pos).unwrap_err();
    assert_eq!(open.code, "E_FIELD_PRESENCE");
    assert_eq!(
        collection_types::static_record_field_type(&T::Map(Box::new(T::String), Box::new(T::Int)),
            "x-y", row.value.pos).unwrap_err().code,
        "E_TYPE"
    );
}
