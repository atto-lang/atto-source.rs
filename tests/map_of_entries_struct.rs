//! Typed Entry-only map construction and source-local nominal Struct audit.
//! Compiler frontend and guest emission are both checked here. A source-only
//! successful type check must not be presented as a runtime execution proof.

use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types::{self, LiteralType},
    compile, lexer, parser::Parser,
};

fn parsed(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("valid lexer"))
        .parse_program().expect("valid grammar")
}
fn admitted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).expect("deterministic compiled guest"));
}
fn rejected(source: &str, code: &str) {
    let error = compile(source).expect_err("unsafe or malformed source emitted Wasm");
    assert_eq!(error.code, code, "{source}\n{error}");
}

#[test]
fn explicit_entry_factory_has_a_real_typed_map_inventory() {
    let source = r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(
            5 => true,
            8 => false,
            9 => true
        );
        return m[5] ? 42 : 0;
    }"#;
    let tree = parsed(source);
    assert!(matches!(&tree.funcs[0].body[0], Stmt::Bind {init, ..}
        if matches!(&init.kind, ExprKind::Call(name, items)
            if name == "@builtin/Map.of_entries<int,bool>" && items.len() == 6)));
    let typed = collection_types::analyze(&tree).expect("Map key/value static type witnesses");
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].value.ty, LiteralType::Map(
        Box::new(LiteralType::Int), Box::new(LiteralType::Bool)));
    admitted(source);
}

#[test]
fn map_entry_sources_are_not_pair_values() {
    admitted(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(
            Entry.of(5, true),
            Entry.of(8, false),
            9 => true
        );
        return m[8] ? 0 : 42;
    }"#);
    admitted(r#"pub fnc main(): int {
        const m = Map.of_entries<string,int>(
            "a" => 40,
            "b" => 2
        );
        return m["a"] + m["b"];
    }"#);
    admitted(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>();
        return 42;
    }"#);
    admitted(r#"pub fnc main(): int {
        val m = Map.of_entries<int,bool>(5 => true);
        return m[5] ? 42 : 0;
    }"#);
}

#[test]
fn entry_arities_ambiguity_and_type_mismatches_fail_closed() {
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5, true);
        return 0;
    }"#, "E_FACTORY_ARITY");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(Pair.of(5,true));
        return 0;
    }"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5 => true, 5 => false);
        return 0;
    }"#, "E_DUPLICATE");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>("5" => true);
        return 0;
    }"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5 => 42);
        return 0;
    }"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<float,bool>(5.0 => true);
        return 0;
    }"#, "E_MAP_KEY");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5 => true, 0 => true, -0 => false);
        return 0;
    }"#, "E_DUPLICATE");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries(5 => true);
        return 0;
    }"#, "E_SYNTAX");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int>(5 => true);
        return 0;
    }"#, "E_SYNTAX");
}

#[test]
fn static_const_does_not_bless_unsafe_dynamic_values() {
    rejected(r#"pub fnc main(): int {
        val bool b = true;
        const m = Map.of_entries<int,bool>(5 => b);
        return 0;
    }"#, "E_UNSUPPORTED");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5 => true);
        return m[6] ? 1 : 0;
    }"#, "E_UNSUPPORTED");
    rejected(r#"pub fnc main(): int {
        const m = Map.of_entries<int,bool>(5 => true,6 => false,7 => true,
                                           8 => false,9 => true);
        return 0;
    }"#, "E_UNSUPPORTED");
}

#[test]
fn declared_structs_preserve_nominal_identity_and_member_layout() {
    admitted(r#"define struct Point as
        int x;
        int y;
    end
    define struct Offset as
        int x;
        int y;
    end
    pub fnc main(): int {
        val Point point = Point{y: 2,x: 40};
        val Offset offset = Offset{x: 10,y: 2};
        return point.x + offset.y;
    }"#);
    admitted(r#"define struct Point as int x; int y; end
    pub fnc main(): int {
        val p = infer struct{y: 2,x: 40};
        return p.x + p.y;
    }"#);
    admitted(r#"define struct Data as int x; bool ready; end
    pub fnc main(): int {
        val Data d = Data{ready: true,x: 42};
        return d.ready ? d.x : 0;
    }"#);
}

#[test]
fn struct_conflicts_duplicates_and_unsafe_field_types_reject() {
    rejected(r#"define struct First as int x; int y; end
    define struct Second as int x; int y; end
    pub fnc main(): int {
        val x = infer struct{x: 40,y: 2};
        return 0;
    }"#, "E_AMBIGUOUS_MEMBER");
    rejected(r#"define struct Point as int x; int y; end
    pub fnc main(): int {
        val p = Point{x: 40};
        return 0;
    }"#, "E_SHAPE");
    rejected(r#"define struct Point as int x; int y; end
    pub fnc main(): int {
        val p = Point{x: 40,y: 2,z: 3};
        return 0;
    }"#, "E_SHAPE");
    rejected(r#"define struct Point as int x; int x; end
    pub fnc main(): int {return 0;}"#, "E_DUPLICATE");
    rejected(r#"define struct Point as List<int> data; end
    pub fnc main(): int {return 0;}"#, "E_UNSUPPORTED_TYPE");
    rejected(r#"define struct Empty as end
    pub fnc main(): int {return 0;}"#, "E_UNSUPPORTED");
    rejected(r#"define struct First as int x; end
    define struct Second as int x; end
    pub fnc main(): int {
        val Second s = First{x: 42};
        return 0;
    }"#, "E_TYPE");
}
