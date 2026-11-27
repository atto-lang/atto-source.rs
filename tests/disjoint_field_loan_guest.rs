//! Compiler-level regression for disjoint-field borrowing and bounded
//! non-lexical lifetimes. This test calls the *real compiler* in-process and
//! validates output bytes and typed diagnostics. Execution/observable guest
//! values are independently checked by tests/conformance.mjs (native+wasm32).
use atto_compiler::compile;

fn accepted(name: &str, source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{name}: not a Wasm guest");
    assert_eq!(compile(source).unwrap(), wasm, "{name}: nondeterministic Wasm");
}

fn rejected(name: &str, source: &str, diagnostic: &str) {
    let err = compile(source).expect_err(name);
    assert_eq!(err.code, diagnostic, "{name}: {err}");
    // compile() returns a Result; on error no guest bytes are returned.
}

#[test]
fn disjoint_scalar_field_borrows_compile_to_guest_wasm() {
    for (name, src) in [
        ("name_vs_age", include_str!("fixtures/pass/field-loan-disjoint-name-age.atto")),
        ("age_vs_name", include_str!("fixtures/pass/field-loan-disjoint-other-direction.atto")),
        ("two_readers", include_str!("fixtures/pass/field-loan-two-readers-same-owner.atto")),
        ("last_use_same_field", include_str!("fixtures/pass/field-loan-nll-same-field-after-last-use.atto")),
        ("branch_join", include_str!("fixtures/pass/field-loan-nll-branch-join.atto")),
        ("class_reader_then_writer", include_str!("fixtures/pass/field-loan-nll-whole-class-reader-then-writer.atto")),
        ("copy_after_last_mut_use", include_str!("fixtures/pass/field-loan-nll-copy-after-last-mut-use.atto")),
        ("copy_after_unused_mut", include_str!("fixtures/pass/field-loan-nll-copy-after-unused-mut.atto")),
    ] {
        accepted(name, src);
    }
}

#[test]
fn overlapping_or_unauthenticated_field_operations_fail_closed() {
    for (name, src, diagnostic) in [
        ("overlapping_write", include_str!("fixtures/fail/field-loan-reject-overlap.atto"), "E_BORROW"),
        ("whole_owner", include_str!("fixtures/fail/field-loan-reject-whole-owner.atto"), "E_BORROW"),
        ("unknown_method", include_str!("fixtures/fail/field-loan-reject-opaque-receiver-method.atto"), "E_BORROW"),
        ("mutable_projection_no_abi", include_str!("fixtures/fail/field-loan-reject-mut-scalar-projection.atto"), "E_BORROW"),
        ("owner_move", include_str!("fixtures/fail/field-loan-reject-owner-take.atto"), "E_BORROW"),
        ("unproved_borrow_root", include_str!("fixtures/fail/field-loan-reject-unverified-alias-root.atto"), "E_BORROW"),
        ("unproved_write_root", include_str!("fixtures/fail/field-loan-reject-disjoint-on-unverified-owner.atto"), "E_OWNERSHIP"),
        ("later_branch_use", include_str!("fixtures/fail/field-loan-nll-reject-later-branch-use.atto"), "E_BORROW"),
        ("class_alias_cannot_take", include_str!("fixtures/fail/field-loan-nll-borrowed-class-never-becomes-owner.atto"), "E_BORROW"),
        ("rhs_moves_owner", include_str!("fixtures/fail/field-loan-reject-owner-moved-during-rhs.atto"), "E_MOVED"),
        ("copy_doublebang_exclusive_loan", include_str!("fixtures/fail/996-copy-doublebang-exclusive-loan.atto"), "E_BORROW"),
    ] {
        rejected(name, src, diagnostic);
    }
}
