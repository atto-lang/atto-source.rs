//! Real compiled guest allocation for the bounded, static-key Map.of subset.
//! Not a dynamic hash table, mutation API or cross-tenant Map transport proof.

use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types::{self, LiteralType},
    compile, lexer,
    parser::Parser,
    scalar_map_guest,
};

fn admitted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).expect("deterministic compiled guest"));
}

fn rejected(source: &str, code: &str) {
    let err = compile(source).expect_err("unsupported Map operation emitted Wasm");
    assert_eq!(err.code, code, "{source}\n{err}");
}

#[test]
fn real_guest_map_forms_and_constant_keys() {
    admitted(
        r#"pub fnc main(): int {
        val m = Map.of("a", 40, "b", 2);
        return m["a"] + m["b"];
    }"#,
    );
    admitted(
        r#"pub fnc main(): int {
        val m = Map.of("a" => 40, "b" => 2);
        return m["b"] + m["a"];
    }"#,
    );
    admitted(
        r#"pub fnc main(): int {
        val m = Map.of(1, 40, -2, 2);
        return m[1] + m[-2];
    }"#,
    );
    admitted(
        r#"pub fnc main(): int {
        val m = Map.of(true, 40, false, 2);
        return m[true] + m[false];
    }"#,
    );
}

#[test]
fn actual_original_five_factory_program_compiles_to_guest_wasm() {
    admitted(
        r#"pub fnc main(): int {
        val pair = Tuple.of(10, 20);
        val values = Array.of(1, 2, 3);
        val names = List.of("Atto", "Lima");
        val scores = Map.of("a", 10, "b", 20);
        val item = Record.of(name="Atto", count=3);
        return values[0] + pair[1];
    }"#,
    );
}

#[test]
fn closed_map_key_type_and_owned_guest_storage_are_checked() {
    let source = r#"pub fnc main(): int {
        val scores = Map.of("a", 40, "b", 2);
        return scores["a"] + scores["b"];
    }"#;
    let parsed = Parser::new(lexer::lex(source).expect("lex")).parse_program().expect("parse");
    let typed = collection_types::analyze(&parsed).expect("static Map types");
    assert_eq!(
        typed[0].value.ty,
        LiteralType::Map(Box::new(LiteralType::String), Box::new(LiteralType::Int))
    );
    let mut lowered = parsed.clone();
    scalar_map_guest::lower(&mut lowered, &typed).expect("checked Map guest layout");
    assert_eq!(lowered.classes.len(), parsed.classes.len() + 1);
    assert_eq!(lowered.classes[0].fields.len(), 4);
    assert!(matches!(&lowered.funcs[0].body.last(),
        Some(Stmt::Return(Some(expr), _)) if matches!(&expr.kind, ExprKind::Binary(..))));
    admitted(source);
}

#[test]
fn duplicate_keys_type_mismatches_and_unsupported_key_forms_reject() {
    rejected(r#"pub fnc main(): int {val m=Map.of("a",1,"a",2);return 0;}"#, "E_DUPLICATE");
    rejected(r#"pub fnc main(): int {val m=Map.of(0,1,-0,2);return 0;}"#, "E_DUPLICATE");
    rejected(r#"pub fnc main(): int {val m=Map.of("a",1,2,3);return 0;}"#, "E_TYPE");
    rejected(r#"pub fnc main(): int {val m=Map.of("a",1,"b");return 0;}"#, "E_FACTORY_ARITY");
    rejected(r#"pub fnc main(): int {val m=Map.of("a"=>1,"b",2);return 0;}"#, "E_FACTORY_ARITY");
    rejected(r#"pub fnc main(): int {val m=Map.of();return 0;}"#, "E_TYPE_INFERENCE");
}

#[test]
fn dynamic_access_mutable_borrow_and_excess_capacity_fail_closed() {
    rejected(
        r#"pub fnc main(): int {
        val m=Map.of("a",42);val key="a";return m[key];
    }"#,
        "E_UNSUPPORTED",
    );
    rejected(
        r#"pub fnc main(): int {val m=Map.of("a",42);return m["missing"];}"#,
        "E_UNSUPPORTED",
    );
    rejected(
        r#"pub fnc main(): int {val m=Map.of("a",42);val writer=borrow! mut m;return 0;}"#,
        "E_BORROW",
    );
    rejected(
        r#"pub fnc main(): int {val m=Map.of("a",1,"b",2,"c",3,"d",4,"e",5);return 0;}"#,
        "E_UNSUPPORTED",
    );
    rejected(r#"pub fnc main(): int {const m=Map.of("a",1);return 0;}"#, "E_UNSUPPORTED");
    rejected(
        r#"pub fnc main(): int {val m=Map.of("a",Map.of("b",42));return 0;}"#,
        "E_UNSUPPORTED",
    );
}
