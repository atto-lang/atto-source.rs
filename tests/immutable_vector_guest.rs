//! Bounded compiled immutable Vector<T> scalar-value guest profile.
//! Explicitly not a dynamic persistent update implementation.

use atto_compiler::{
    collection_types::{self, LiteralType},
    compile, lexer, parser::Parser,
};

fn accepted(source: &str) {
    let a = compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
    assert!(a.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(a, compile(source).expect("deterministic guest Wasm"));
}

fn denied(source: &str, code: &str) {
    let e = compile(source).expect_err("unsupported Vector emitted a guest artifact");
    assert_eq!(e.code, code, "{source}\n{e}");
}

#[test]
fn canonical_const_vector_empty_is_a_real_typed_immutable_guest_value() {
    accepted("pub fnc main(): int {const Vector<int> foo=[];return 42;}");
    accepted("pub fnc main(): int {const Vector<int> foo=[40,2];return foo[0]+foo[1];}");
    accepted("pub fnc main(): int {Vector<int> foo=[42];return foo[0];}");
}

#[test]
fn vector_is_not_an_alias_for_list_or_array() {
    let source = "pub fnc main(): int {val Vector<int> v=[1,2];val List<int> l=[1,2];val Array<int> a=[1,2];return 42;}";
    let p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let t = collection_types::analyze(&p).unwrap();
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].value.ty, LiteralType::Vector(Box::new(LiteralType::Int)));
    assert_eq!(t[1].value.ty, LiteralType::List(Box::new(LiteralType::Int)));
    assert_eq!(t[2].value.ty, LiteralType::Array(Box::new(LiteralType::Int)));
    accepted(source);
    accepted("pub fnc main(): int {val vector<int> v=[42];return v[0];}");
}

#[test]
fn unsafe_vector_borrows_dynamic_indices_and_oob_fail_closed() {
    denied("pub fnc main(): int {val Vector<int> v=[42];val writer=borrow! mut v;return 0;}", "E_BORROW");
    denied("pub fnc main(): int {val Vector<int> v=[42];return v[1];}", "E_INDEX_BOUNDS");
    denied("pub fnc main(): int {val Vector<int> v=[];return v[0];}", "E_INDEX_BOUNDS");
    denied("pub fnc main(): int {val Vector<int> v=[42];val i=0;return v[i];}", "E_UNSUPPORTED");
    denied("pub fnc main(): int {val Vector<int> v=[1,2,3,4,5,6,7,8,9];return 0;}", "E_UNSUPPORTED");
}

#[test]
fn constexpr_vector_is_not_a_runtime_readonly_val_fallback() {
    denied("pub fnc main(): int {val int value=42;const Vector<int> v=[value];return 0;}", "E_NON_CONST_INITIALIZER");
    denied("pub fnc main(): int {const Vector<int> v=[1,\"bad\"];return 0;}", "E_TYPE");
    denied("pub fnc main(): int {const Vector<int> v=Array.of(42);return 0;}", "E_NON_CONST_INITIALIZER");
    denied("pub fnc main(): int {const Vector<int> v=[1,2,3,4,5,6,7,8,9];return 0;}", "E_UNSUPPORTED");
    denied("pub fnc main(): int {val Vector<int> v=[42];v= [1];return 0;}", "E_IMMUTABLE_REBIND");
}
