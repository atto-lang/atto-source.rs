//! Contextual empty collections, named Record tuples and Pair/Entry arrows.
//! This test separates type-only optional tuples from executable scalar guest
//! storage; no host-only simulation is treated as a guest proof.

use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types::{self, LiteralType as T},
    compile, lexer, parser::Parser,
};

fn parsed(body: &str) -> atto_compiler::ast::Program {
    let source = format!("pub fnc main(): int {{\n{body}\nreturn 0;\n}}");
    Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap()
}

fn typed(body: &str) -> Vec<collection_types::TypedBinding> {
    collection_types::analyze(&parsed(body)).expect("contextual type preflight")
}

fn guest(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).unwrap(), "deterministic guest bytes");
}

fn rejected(source: &str, code: &str) {
    let error = compile(source).expect_err("unsupported program emitted guest bytes");
    assert_eq!(error.code, code, "{source}\n{error}");
}

#[test]
fn typed_empty_list_and_map_have_distinct_checked_shapes() {
    let info = typed(r#"
        val List<int> xs = [];
        val Map<int,string> attrs = {};
    "#);
    assert_eq!(info.len(), 2);
    assert_eq!(info[0].value.ty, T::List(Box::new(T::Int)));
    assert_eq!(info[1].value.ty, T::Map(Box::new(T::Int),Box::new(T::String)));
    guest(r#"pub fnc main(): int {
        val List<int> xs = [];
        val Map<int,string> attrs = {};
        return 42;
    }"#);
    guest(r#"pub fnc main(): int {
        val Array<string> xs = [];
        return 42;
    }"#);
}

#[test]
fn optional_tuple_components_and_none_are_type_checked_without_guest_pretence() {
    let program = parsed(r#"
        val Tuple[int?,string?,bool?] defaults = (None,None,None);
    "#);
    assert!(matches!(&program.funcs[0].body[0], Stmt::Bind {
        init: atto_compiler::ast::Expr { kind: ExprKind::Tuple(items), ..}, ..
    } if items.len() == 3));
    let shape = collection_types::analyze(&program).expect("typed None positions");
    assert_eq!(shape[0].value.ty, T::Tuple(vec![
        T::Option(Box::new(T::Int)),
        T::Option(Box::new(T::String)),
        T::Option(Box::new(T::Bool)),
    ]));
    rejected(r#"pub fnc main(): int {
        val Tuple[int?,string?,bool?] defaults = (None,None,None);
        return 0;
    }"#, "E_UNSUPPORTED");
    rejected(r#"pub fnc main(): int {
        val Tuple<int?,bool?> defaults = (None,None,None);
        return 0;
    }"#, "E_TYPE_ARITY");
    rejected(r#"pub fnc main(): int {
        val t = (None,None);
        return 0;
    }"#, "E_TYPE_INFERENCE");
}

#[test]
fn named_record_tuple_is_closed_record_not_positional_tuple() {
    let program = parsed(r#"val Record r = (abc="foo", xyz=true, foo=5);"#);
    assert!(matches!(&program.funcs[0].body[0], Stmt::Bind {
        init: atto_compiler::ast::Expr { kind: ExprKind::Record(fields), ..}, ..
    } if fields.len()==3 && fields[0].0 == "abc" && fields[1].0 == "xyz"));
    guest(r#"pub fnc main(): int {
        val Record r = (abc="foo", xyz=true, foo=5);
        return r.foo;
    }"#);
    guest(r#"pub fnc main(): int {
        val r = ("content-type"="json", abc=42);
        return r.abc;
    }"#);
    rejected(r#"pub fnc main(): int {
        val r = (abc=1, abc=2);
        return 0;
    }"#, "E_DUPLICATE");
}

#[test]
fn pair_and_entry_are_distinct_types_with_explicit_contextual_arrow() {
    let cases = typed(r#"
        val Pair<int,bool> pair = 5 => true;
        val Entry<int,bool> entry = 5 => true;
        val Pair<int,bool> explicit = Pair.of(5,true);
        val Entry<int,bool> entry_factory = Entry.of(5,true);
    "#);
    assert_eq!(cases.len(),4);
    for i in [0,2] {
        assert_eq!(cases[i].value.ty, T::Pair(Box::new(T::Int),Box::new(T::Bool)));
    }
    for i in [1,3] {
        assert_eq!(cases[i].value.ty, T::Entry(Box::new(T::Int),Box::new(T::Bool)));
    }
    guest(r#"pub fnc main(): int {
        val Pair<int,bool> pair = 5 => true;
        val Entry<int,bool> entry = 40 => true;
        return pair.first + entry.key - 3;
    }"#);
    guest(r#"pub fnc main(): int {
        val Pair<int,bool> pair = Pair.of(40,true);
        val Entry<int,bool> entry = Entry.of(2,false);
        return pair.first + entry.key;
    }"#);
    rejected(r#"pub fnc main(): int {
        val Pair<int,bool> pair = Entry.of(5,true);
        return 0;
    }"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {
        val Entry<int,bool> entry = Pair.of(5,true);
        return 0;
    }"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {
        val Pair<int,bool> pair = 5 => 42;
        return 0;
    }"#, "E_TYPE");
}

#[test]
fn insufficient_context_and_unsupported_dynamic_products_fail_closed() {
    rejected(r#"pub fnc main(): int {val x=[];return 0;}"#, "E_INFER");
    rejected(r#"pub fnc main(): int {val x={};return 0;}"#, "E_INFER");
    rejected(r#"pub fnc main(): int {val Map m={};return 0;}"#, "E_INFER");
    rejected(r#"pub fnc main(): int {val List x=[];return 0;}"#, "E_TYPE_INFERENCE");
    rejected(r#"pub fnc main(): int {val Pair<int,bool> = 5 => true;return 0;}"#, "E_SYNTAX");
    rejected(r#"pub fnc main(): int {val Entry<int,bool> = 5 => true;return 0;}"#, "E_SYNTAX");
    rejected(r#"pub fnc main(): int {val x=5=>true;return 0;}"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {val List<int> x=[1,"str"];return 0;}"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {val Map<int,string> x={"a"=> "wrong"};return 0;}"#, "E_TYPE");
}
