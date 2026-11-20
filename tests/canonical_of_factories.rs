//! Canonical factory frontend + narrow real closed-Record Wasm guest admission.
//! No claim of executable Map, Array, List or polymorphic Record rows.
//! The selected compiler branch deliberately fails closed on unavailable ABI.

use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types::{self, LiteralType},
    compile, lexer,
    parser::Parser,
};

fn parsed(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("lex"))
        .parse_program()
        .expect("parse")
}
fn rejected(source: &str, code: &str) {
    let err = compile(source).expect_err("invalid factory cannot emit guest");
    assert_eq!(err.code, code, "{source}\n{err}");
}

#[test]
fn record_of_named_and_colon_fields_reuse_checked_guest_abi() {
    let source = r#"
        pub fnc main(): int {
          val r = Record.of(name="Atto", count=40, ready: true);
          val h = Record.of("x-y": 2, extra=false);
          return r.count + h["x-y"];
        }
    "#;
    let wasm = compile(source).expect("canonical closed Record should compile");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).expect("deterministic guest"));
}

#[test]
fn canonical_factories_preserve_typed_aggregate_nodes_and_source_order() {
    let p = parsed(
        r#"
        pub fnc main(): int {
          val t = Tuple.of(10, 20);
          val m = Map.of("a", 10, "b", 20);
          val r = Record.of(z=3, a=4);
          return 0;
        }
    "#,
    );
    assert!(matches!(&p.funcs[0].body[0],
        Stmt::Bind {init,..} if matches!(&init.kind, ExprKind::Tuple(items) if items.len()==2)));
    assert!(matches!(&p.funcs[0].body[1],
        Stmt::Bind {init,..} if matches!(&init.kind, ExprKind::Map(items) if items.len()==2)));
    assert!(matches!(&p.funcs[0].body[2],
        Stmt::Bind {init,..} if matches!(&init.kind, ExprKind::Record(items)
            if items.iter().map(|(n,_)|n.as_str()).collect::<Vec<_>>() == vec!["z","a"])));
    let t = collection_types::analyze(&p).expect("factory typed preflight");
    assert!(matches!(&t[0].value.ty, LiteralType::Tuple(items) if items.len()==2));
    assert!(matches!(&t[1].value.ty, LiteralType::Map(_, _)));
    assert_eq!(t[2].value.source_to_record_slot, vec![1, 0]);
}

#[test]
fn array_and_list_of_keep_distinct_inferred_types() {
    let p = parsed(
        r#"
        pub fnc main(): int {
          val a = Array.of(1, 2, 3);
          val l = List.of("Atto", "Lima");
          return 0;
        }
    "#,
    );
    let t = collection_types::analyze(&p).expect("distinct factory type inference");
    assert_eq!(
        t[0].value.ty,
        LiteralType::Array(Box::new(LiteralType::Int))
    );
    assert_eq!(
        t[1].value.ty,
        LiteralType::List(Box::new(LiteralType::String))
    );
}

#[test]
fn array_list_and_map_guest_execution_do_not_acquire_a_host_fallback() {
    rejected(
        r#"pub fnc main(): int { val m = Map.of("a",1); return 0; }"#,
        "E_UNSUPPORTED",
    );
    rejected(
        r#"pub fnc main(): int { val a = Array.of(1,2,3); return 0; }"#,
        "E_UNSUPPORTED",
    );
    rejected(
        r#"pub fnc main(): int { val l = List.of("a","b"); return 0; }"#,
        "E_UNSUPPORTED",
    );
    rejected(
        r#"pub fnc main(): int { val x = Array.of(); return 0; }"#,
        "E_TYPE_INFERENCE",
    );
    rejected(
        r#"pub fnc main(): int { val x = List.of(); return 0; }"#,
        "E_TYPE_INFERENCE",
    );
    rejected(
        r#"pub fnc main(): int { val x = Array.of(1,"two"); return 0; }"#,
        "E_TYPE_INFERENCE",
    );
}

#[test]
fn duplicate_fields_and_keys_and_bad_arity_reject() {
    rejected(
        r#"pub fnc main(): int { val r=Record.of(name=1,"name":2); return 0; }"#,
        "E_DUPLICATE",
    );
    rejected(
        r#"pub fnc main(): int { val m=Map.of("a",1,"a",2); return 0; }"#,
        "E_DUPLICATE",
    );
    rejected(
        r#"pub fnc main(): int { val m=Map.of("a",1,"b"); return 0; }"#,
        "E_FACTORY_ARITY",
    );
    rejected(
        r#"pub fnc main(): int { val m=Map.of("a" => 1, "b", 2); return 0; }"#,
        "E_FACTORY_ARITY",
    );
    rejected(
        r#"pub fnc main(): int { val r=Record.of(key => 1); return 0; }"#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn builtin_factory_identity_cannot_be_shadowed_by_a_user_variable() {
    rejected(
        r#"
        pub fnc main(): int {
          let Record = 7;
          val r = Record.of(value=42);
          return r.value;
        }
    "#,
        "E_RESERVED",
    );
}

#[test]
fn empty_record_factory_is_static_but_not_a_guest_allocation_profile() {
    let p = parsed("pub fnc main(): int { val r = Record.of(); return 0; }");
    let checked = collection_types::analyze(&p).expect("empty closed row is statically typed");
    assert_eq!(checked[0].value.ty, LiteralType::Record(vec![]));
    rejected(
        "pub fnc main(): int { val r = Record.of(); return 0; }",
        "E_UNSUPPORTED",
    );
}
