//! Ownership and resource-budget hardening for the bounded immutable Vector guest.
//! Tests compile the real Atto source and require invalid programs to emit no Wasm.
use atto_compiler::compile;

fn fails(source: &str, code: &str) {
    let err = compile(source).expect_err("unsafe Vector source emitted a guest artifact");
    assert_eq!(err.code, code, "{source}\n{err}");
}

#[test]
fn persistent_static_provenance_has_bounded_live_vector_count() {
    let mut source = String::from("pub fnc main(): int {");
    for index in 0..129 {
        source.push_str(&format!("val Vector<int> v{index}=[1];"));
    }
    source.push_str("return 0;}");
    fails(&source, "E_SOURCE_LIMIT");
}

#[test]
fn persistent_rewrites_have_a_separate_cumulative_scalar_budget() {
    let mut source = String::from(
        "pub fnc main(): int {val Vector<int> root=[1,2,3,4,5,6,7,8];");
    // 65 x 8 copied cells exceeds the 512-scalar compile-time bound.
    for index in 0..65 {
        source.push_str(&format!(
            "val Vector<int> v{index}=root.updated(0,42);"));
    }
    source.push_str("return 0;}");
    fails(&source, "E_SOURCE_LIMIT");
}

#[test]
fn scalar_vector_does_not_retain_dynamic_or_mutable_children() {
    fails("pub fnc main(): int {val int x=1;val Vector<int> v=[x];return 0;}",
        "E_UNSUPPORTED");
    fails("pub fnc main(): int {val Vector<Array<int>> v=[[1]];return 0;}",
        "E_UNSUPPORTED");
    fails("pub fnc main(): int {val Vector<int> v=[1];val Vector<int> copy=v.appended(2);val writer=borrow! mut copy;return 0;}",
        "E_BORROW");
    fails("pub fnc main(): int {let mut Vector<int> v=[1];val writer=borrow! mut v;return 0;}",
        "E_BORROW");
}

#[test]
fn static_update_rejects_invalid_indices_and_unproven_receivers() {
    fails("pub fnc main(): int {val Vector<int> v=[1];val Vector<int> x=v.updated(-1,42);return 0;}",
        "E_INDEX_BOUNDS");
    fails("pub fnc main(): int {val Vector<int> v=[1];val Vector<int> x=v.updated(9223372036854775808,42);return 0;}",
        "E_INDEX_BOUNDS");
    fails("pub fnc main(): int {val List<int> v=[1];val Vector<int> x=v.updated(0,42);return 0;}",
        "E_UNSUPPORTED");
}

#[test]
fn non_mutating_copy_updates_preserve_const_and_let_semantics() {
    for source in [
        "pub fnc main(): int {const Vector<int> a=[1];const Vector<int> b=a.updated(0,42);return b[0];}",
        "pub fnc main(): int {let Vector<int> a=[2];val Vector<int> b=a.appended(40);return a[0]+b[1];}",
    ] {
        let wasm=compile(source).unwrap_or_else(|e| panic!("{source}\n{e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(wasm,compile(source).expect("deterministic guest bytes"));
    }
}
