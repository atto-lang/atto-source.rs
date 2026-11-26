//! Verified mutable scalar field-place loans: the alias reloads the actual
//! owner field, assignment writes through to owned storage, and sibling fields
//! remain independent. No raw pointers, class handles, or dynamic projections.
use atto_compiler::compile;

fn succeeds(name: &str, source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{name}: not WebAssembly");
    assert_eq!(compile(source).unwrap(), wasm, "{name}: unstable Wasm emission");
}

fn rejects(name: &str, source: &str, diagnostic: &str) {
    let err = compile(source).expect_err(name);
    assert_eq!(err.code, diagnostic, "{name}: {err}");
}

#[test]
fn scalar_field_mutation_uses_real_parent_storage() {
    for (name, src) in [
        ("int_write", include_str!("fixtures/pass/field-mut-ref-int-write-through.atto")),
        ("str_write", include_str!("fixtures/pass/field-mut-ref-string-write-through.atto")),
        ("disjoint_writers", include_str!("fixtures/pass/field-mut-ref-two-disjoint-writers.atto")),
        ("reload_after_write", include_str!("fixtures/pass/field-mut-ref-read-updated-value.atto")),
        ("shared_sibling", include_str!("fixtures/pass/field-mut-ref-sibling-shared-loan.atto")),
        ("scoped_expiry", include_str!("fixtures/pass/field-mut-ref-scoped-expiry.atto")),
    ] {
        succeeds(name, src);
    }
}

#[test]
fn exclusive_field_loans_fail_closed_when_proofs_are_absent() {
    for (name, src, diagnostic) in [
        ("immutable_owner", include_str!("fixtures/fail/field-mut-ref-reject-immutable-root.atto"), "E_BORROW"),
        ("immutable_field", include_str!("fixtures/fail/field-mut-ref-reject-immutable-field.atto"), "E_IMMUTABLE"),
        ("overlap_read", include_str!("fixtures/fail/field-mut-ref-reject-overlap-owner-read.atto"), "E_BORROW"),
        ("overlap_write", include_str!("fixtures/fail/field-mut-ref-reject-overlap-owner-write.atto"), "E_BORROW"),
        ("two_writer_conflict", include_str!("fixtures/fail/field-mut-ref-reject-two-writers-same-field.atto"), "E_BORROW"),
        ("reader_writer_conflict", include_str!("fixtures/fail/field-mut-ref-reject-shared-exclusive-overlap.atto"), "E_BORROW"),
        ("owner_move", include_str!("fixtures/fail/field-mut-ref-reject-owner-take.atto"), "E_BORROW"),
        ("immutable_alias", include_str!("fixtures/fail/field-mut-ref-reject-val-alias.atto"), "E_BORROW"),
        ("nested_class_unimplemented", include_str!("fixtures/fail/field-mut-ref-reject-nested-class-target.atto"), "E_UNSUPPORTED"),
        ("nested_projection_no_proof", include_str!("fixtures/fail/field-mut-ref-reject-nested-projection.atto"), "E_BORROW"),
    ] {
        rejects(name, src, diagnostic);
    }
}
