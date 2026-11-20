//! Map.of_entries<K,V>: explicit Entry arguments with a checked Map witness.
//! Execution is bounded to the existing owned scalar guest-map subset.
//! It does not authorize variable Entry consumption, dynamic hash lookup,
//! general Map resize, cross-tenant sharing, or runtime const allocation.

use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types::{self, LiteralType},
    compile, lexer, parser::Parser,
};

fn checked(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).unwrap()).parse_program()
        .unwrap_or_else(|err| panic!("{source}\n{err}"))
}

fn admitted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|err| panic!("{source}\n{err}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).unwrap(), "guest compilation is deterministic");
}

fn rejected(source: &str, code: &str) {
    let err = compile(source).expect_err("invalid Map.of_entries emitted Wasm");
    assert_eq!(err.code, code, "{source}\n{err}");
}

#[test]
fn explicit_types_preserve_kv_shape_and_source_pair_order() {
    let source = r#"pub fnc main(): int {
        val m = Map.of_entries<int, bool>(5 => true, 8 => false, 9 => true);
        return 42;
    }"#;
    let p = checked(source);
    let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else {panic!("first statement not binding")};
    assert!(matches!(&init.kind, ExprKind::Call(name, args)
        if name == "@builtin/Map.of_entries<int,bool>" && args.len() == 6));
    let typed = collection_types::analyze(&p).unwrap();
    assert_eq!(typed[0].value.ty, LiteralType::Map(
        Box::new(LiteralType::Int), Box::new(LiteralType::Bool)));
    assert_eq!(typed[0].value.children.len(), 6);
    admitted(source);
}

#[test]
fn static_entries_and_explicit_entry_factories_compile_in_guest() {
    admitted(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(
            5 => true,
            8 => false,
            9 => true,
        );
        return m[5] == true ? 42 : 0;
    }"#);
    admitted(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(
            Entry.of(5,true), Entry.of(8,false), Entry.of(9,true)
        );
        return m[9] == true ? 42 : 0;
    }"#);
    admitted(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(5 => true, Entry.of(8,false));
        return m[5] == true ? 42 : 0;
    }"#);
    admitted(r#"pub fnc main(): int {
        val Map<int,bool> m = Map.of_entries(5 => true,8 => false);
        return m[5] == true ? 42 : 0;
    }"#);
    admitted(r#"pub fnc main(): int {
        val m = Map.of_entries(5 => true,8 => false);
        return m[5] == true ? 42 : 0;
    }"#);
    admitted(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>();
        return 42;
    }"#);
}

#[test]
fn malformed_entry_arguments_and_duplicate_keys_reject() {
    rejected(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(5,true);
        return 0;
    }"#,"E_UNSUPPORTED");
    rejected(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(5 => true,5 => false);
        return 0;
    }"#,"E_DUPLICATE");
    rejected(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(0 => true,-0 => false);
        return 0;
    }"#,"E_DUPLICATE");
    rejected(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(5 => true,8 => "wrong");
        return 0;
    }"#,"E_TYPE");
    rejected(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>("5" => true);
        return 0;
    }"#,"E_TYPE");
    rejected(r#"pub fnc main(): int {
        val Map<int,string> m = Map.of_entries<int,bool>(5 => true);
        return 0;
    }"#,"E_TYPE");
}

#[test]
fn owned_entry_variables_dynamic_keys_and_runtime_const_remain_fail_closed() {
    rejected(r#"pub fnc main(): int {
        val Entry<int,bool> e = 5 => true;
        val m = Map.of_entries<int,bool>(e);
        return 0;
    }"#,"E_UNSUPPORTED");
    rejected(r#"pub fnc main(): int {
        val int k = 5;
        val m = Map.of_entries<int,bool>(k => true);
        return 0;
    }"#,"E_UNSUPPORTED");
    rejected(r#"pub fnc main(): int {
        val m = Map.of_entries();
        return 0;
    }"#,"E_TYPE_INFERENCE");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5 => true);
        return 0;
    }"#,"E_UNSUPPORTED");
}
