//! Executable first bounded scalar Tuple.of/Array.of guest ABI.
//! This does not claim dynamic-index or homogeneous List runtime support.

use atto_compiler::{
    ast::{ExprKind, Stmt},
    collection_types::{self, LiteralType},
    compile, fixed_sequence_guest, lexer,
    parser::Parser,
};

fn admitted(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(source).expect("deterministic module bytes"));
}
fn rejected(source: &str, expected: &str) {
    let err = compile(source).expect_err("invalid sequence must not emit guest module");
    assert_eq!(err.code, expected, "{source}\n{err}");
}

#[test]
fn real_guest_array_and_tuple_const_index_fixtures() {
    admitted("pub fnc main(): int {val pair=Tuple.of(10,20);val values=Array.of(1,2,3);return values[0]+pair[1];}");
    admitted("pub fnc main(): int {val t=Tuple.of(40,true,\"ok\");return t[0];}");
    admitted("pub fnc main(): int {val a=Array.of(20,22);return a[0]+a[1];}");
}

#[test]
fn allocated_field_abi_is_private_and_has_proven_exact_length() {
    let source =
        "pub fnc main(): int {val t=Tuple.of(40,true);val a=Array.of(1,2,3);return t[0]+a[1];}";
    let program = Parser::new(lexer::lex(source).unwrap())
        .parse_program()
        .unwrap();
    let typed = collection_types::analyze(&program).unwrap();
    assert_eq!(
        typed[0].value.ty,
        LiteralType::Tuple(vec![LiteralType::Int, LiteralType::Bool])
    );
    assert_eq!(
        typed[1].value.ty,
        LiteralType::Array(Box::new(LiteralType::Int))
    );
    let mut lowered = program.clone();
    fixed_sequence_guest::lower(&mut lowered, &typed).unwrap();
    assert_eq!(lowered.classes.len(), program.classes.len() + 2);
    assert_eq!(lowered.classes[0].fields.len(), 2);
    assert_eq!(lowered.classes[1].fields.len(), 3);
    assert!(lowered
        .classes
        .iter()
        .all(|cls| cls.name.starts_with(fixed_sequence_guest::PRIVATE_PREFIX)));
    assert!(lowered.classes[0].fields[0].name.starts_with('\u{1f}'));
    assert!(matches!(&lowered.funcs[0].body.last().unwrap(),
        Stmt::Return(Some(expr), _) if matches!(&expr.kind, ExprKind::Binary(..))));
    admitted(source);
}

#[test]
fn unsafe_guest_indexing_and_element_shapes_fail_closed() {
    rejected(
        "pub fnc main(): int {val a=Array.of(1,2);return a[2];}",
        "E_INDEX_BOUNDS",
    );
    rejected(
        "pub fnc main(): int {val t=Tuple.of(10);return t[-1];}",
        "E_INDEX_BOUNDS",
    );
    rejected(
        "pub fnc main(): int {val t=Tuple.of(10);val i=0;return t[i];}",
        "E_UNSUPPORTED",
    );
    rejected(
        "pub fnc main(): int {val a=Array.of(1,2);return a[\"0\"];}",
        "E_UNSUPPORTED",
    );
    rejected(
        "pub fnc main(): int {val a=Array.of(1,\"bad\");return 0;}",
        "E_TYPE_INFERENCE",
    );
    rejected(
        "pub fnc main(): int {val a=Array.of();return 0;}",
        "E_TYPE_INFERENCE",
    );
    rejected(
        "pub fnc main(): int {val t=Tuple.of();return 0;}",
        "E_UNSUPPORTED",
    );
    rejected(
        "pub fnc main(): int {val a=Array.of(1,2,3,4,5,6,7,8,9);return 0;}",
        "E_UNSUPPORTED",
    );
}

#[test]
fn ownership_and_synthetic_type_forgery_fail_closed() {
    rejected(
        "pub fnc main(): int {val a=Array.of(1,2);val writer=borrow! mut a;return 0;}",
        "E_BORROW",
    );
    rejected(
        "pub fnc main(): int {val t=Tuple.of(42);val writer=borrow! mut t;return 0;}",
        "E_BORROW",
    );
    rejected(
        "pub fnc main(): int {val __atto_fixed_sequence_shape_0=42;return 0;}",
        "E_RESERVED",
    );
    rejected(
        "pub fnc main(): int {const t=Tuple.of(42);return 0;}",
        "E_UNSUPPORTED",
    );
}

#[test]
fn unsupported_sequence_escapes_and_nested_bindings_are_not_lowered() {
    rejected(
        "pub fnc main(): int {val a=Array.of(Array.of(1));return 0;}",
        "E_UNSUPPORTED",
    );
    rejected(
        "pub fnc main(): int {block {val a=Array.of(1,2);} return 0;}",
        "E_UNSUPPORTED",
    );
    admitted("pub fnc main(): int {val x=List.of(20,22);return x[0]+x[1];}");
    admitted("pub fnc main(): int {val x=Map.of(\"a\",42);return x[\"a\"];}");
}

#[test]
fn bounded_list_guests_have_checked_index_and_immutable_storage() {
    admitted("pub fnc main(): int {val names=List.of(\"Atto\",\"Lima\");stdio.println(names[0]);return 42;}");
    admitted("pub fnc main(): int {val scores=List.of(20,22);return scores[0]+scores[1];}");
    rejected("pub fnc main(): int {val names=List.of(10,20);return names[2];}", "E_INDEX_BOUNDS");
    rejected("pub fnc main(): int {val x=List.of(10,20);val i=1;return x[i];}", "E_UNSUPPORTED");
    rejected("pub fnc main(): int {val x=List.of(10,20);val writer=borrow! mut x;return 0;}", "E_BORROW");
    rejected("pub fnc main(): int {val x=List.of(1,\"two\");return 0;}", "E_TYPE_INFERENCE");
    rejected("pub fnc main(): int {val x=List.of();return 0;}", "E_TYPE_INFERENCE");
    rejected("pub fnc main(): int {val x=List.of(1,2,3,4,5,6,7,8,9);return 0;}", "E_UNSUPPORTED");
}
