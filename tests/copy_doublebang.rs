//! Real Rust/Wasm frontend compilation gates for canonical copy!! syntax.
//! Identity-free scalars and verified FLAT class copies compile to Wasm.
//! Nested mutable graph, Sendable, inheritance and actor-heap copies remain
//! fail closed until distinct proof and code-generation machinery exists.
use atto_compiler::compile;

fn wasm(source: &str) -> Vec<u8> {
    let bytes = compile(source).unwrap_or_else(|d| panic!("{source}: {d}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(compile(source).unwrap(), bytes, "deterministic source lowering");
    bytes
}

#[test]
fn scalar_deep_copy_produces_real_wasm_bytes() {
    wasm("pub fnc main(): int { val x = 41; val y = copy!! x; return y + 1; }");
    wasm("pub fnc main(): int { return copy!!(42); }");
}

#[test]
fn copy_bang_then_space_bang_is_not_the_deep_operator() {
    // The contiguous double-bang spelling is the deep operation; this
    // spaced spelling must remain shallow-copy of a logical negation.
    wasm("pub fnc main(): int { val flag = copy! !false; return flag ? 42 : 0; }");
}

#[test]
fn generated_flat_class_copier_emits_real_wasm_bytes() {
    wasm(include_str!("fixtures/pass/993-copy-doublebang-flat-class.atto"));
    wasm(include_str!("fixtures/pass/994-copy-doublebang-two-independent-class-copies.atto"));
    wasm(include_str!("fixtures/pass/995-copy-doublebang-flat-class-mixed-fields.atto"));
    wasm(include_str!("fixtures/pass/996-copy-doublebang-constructor-state.atto"));
    wasm(include_str!("fixtures/pass/997-copy-doublebang-private-state.atto"));
    wasm("define class Parcel as pub let int value = 1; end pub fnc main(): int { val q = copy!! (new Parcel()); return q.value; }");
}

#[test]
fn an_unverified_custom_copy_deep_method_fails_closed() {
    let src = include_str!("fixtures/fail/991-copy-doublebang-class-unproved.atto");
    let diagnostic = compile(src).expect_err("user copy_deep override is not yet verified");
    assert_eq!(diagnostic.code, "E_COPY_OVERRIDE", "{diagnostic}");
}

#[test]
fn copied_class_cannot_be_read_during_exclusive_mutable_loan() {
    let src = include_str!("fixtures/fail/996-copy-doublebang-exclusive-loan.atto");
    let diagnostic = compile(src).expect_err("cannot read an exclusive borrowed owner");
    assert_eq!(diagnostic.code, "E_BORROW", "{diagnostic}");
}

#[test]
fn nested_array_copy_fails_closed_before_codegen() {
    let source = include_str!("fixtures/fail/997-copy-doublebang-nested-array-no-runtime.atto");
    let diagnostic = compile(source).expect_err("Array<Array<int>> requires a checked allocation/index ABI");
    assert_eq!(diagnostic.code, "E_UNSUPPORTED_TYPE", "{diagnostic}");
}

#[test]
fn copy_deep_disallows_mut_qualifier_and_missing_operand() {
    let diagnostic = compile("pub fnc main(): int { val x = 1; return copy!! mut x; }").expect_err("mut qualifier is reserved for borrow!");
    assert_eq!(diagnostic.code, "E_SYNTAX");
    assert!(compile("pub fnc main(): int { return copy!!; }").is_err());
}

#[test]
fn separate_bangs_and_spelling_have_distinct_parsed_behavior() {
    // Both witnesses must be independently derived for the closed flat layout.
    let prefix = "define class Parcel as pub let int value = 1; end pub fnc main(): int { val p = new Parcel(); ";
    let shallow = format!("{prefix}val q = copy! p; return 0; }}");
    wasm(&shallow);
    let deep = format!("{prefix}val q = copy!! p; return q.value; }}");
    wasm(&deep);
}

#[test]
fn deep_copy_after_inferred_mutable_loan_end_is_accepted() {
    wasm(include_str!("fixtures/pass/field-loan-nll-copy-after-unused-mut.atto"));
    wasm(include_str!("fixtures/pass/field-loan-nll-copy-after-last-mut-use.atto"));
}

#[test]
fn shallow_copy_flat_root_and_live_loan_gate() {
    wasm(include_str!("fixtures/pass/998-copy-shallow-flat-class.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/998-copy-shallow-exclusive-loan.atto")).unwrap_err().code, "E_BORROW");
    assert_eq!(compile(include_str!("fixtures/fail/153-copy-noncopy-class.atto")).unwrap_err().code, "E_COPY_OVERRIDE");
}
#[test]
fn shallow_copy_after_last_mutable_loan_use() {
    let source = include_str!("fixtures/pass/field-loan-nll-copy-after-last-mut-use.atto").replace("copy!! original","copy! original");
    wasm(&source);
}

#[test]
fn nested_immutable_leaf_copy_and_projection_boundaries() {
    wasm(include_str!("fixtures/pass/999-copy-immutable-leaf.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/999-copy-nested-handle-escape.atto")).unwrap_err().code, "E_BORROW");
    assert_eq!(compile(include_str!("fixtures/fail/1000-copy-nested-mutable-unproved.atto")).unwrap_err().code, "E_UNSUPPORTED_TYPE");
    assert_eq!(compile(include_str!("fixtures/fail/1001-copy-nested-write.atto")).unwrap_err().code, "E_OWNERSHIP");
}

#[test]
fn closed_mutable_leaf_sharing_and_detachment() {
    wasm(include_str!("fixtures/pass/1002-copy-mutable-detachment.atto"));
    wasm(include_str!("fixtures/pass/1003-copy-mutable-deep.atto"));
    wasm(include_str!("fixtures/pass/1004-copy-mutable-third.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/1005-copy-shared-source-write.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1006-copy-shared-copy-write.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1007-copy-shared-third-write.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1008-copy-mutable-control-flow.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1009-copy-mutable-handle-escape.atto")).unwrap_err().code, "E_BORROW");
}

#[test]
fn lexical_copy_groups_restore_only_last_alias_authority() {
    wasm(include_str!("fixtures/pass/1010-copy-lexical-release.atto"));
    wasm(include_str!("fixtures/pass/1011-copy-lexical-third-release.atto"));
    wasm(include_str!("fixtures/pass/1012-copy-source-recovery-on-detach.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/1013-copy-live-sibling-after-block.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1014-copy-root-transfer.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1015-copy-root-take.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1016-copy-root-borrow.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1017-copy-root-rebind.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1018-copy-untracked-temp.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1019-copy-root-call-boundary.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
}

#[test]
fn fixed_nested_slot_remains_fixed_on_shallow_and_deep_copy() {
    wasm(include_str!("fixtures/pass/1020-copy-fixed-slot.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/1021-copy-fixed-slot-replace.atto")).unwrap_err().code, "E_COPY_READONLY_SLOT");
}

#[test]
fn bounded_fresh_tree_copy_and_transitive_authority() {
    wasm(include_str!("fixtures/pass/1022-copy-tree-detach.atto"));
    wasm(include_str!("fixtures/pass/1023-copy-tree-deep.atto"));
    wasm(include_str!("fixtures/pass/1024-copy-tree-release.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/1025-copy-tree-source-write.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1026-copy-tree-inner-replace.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1027-copy-tree-shared-inner-replace.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1028-copy-tree-cycle.atto")).unwrap_err().code, "E_UNSUPPORTED_TYPE");
    assert_eq!(compile(include_str!("fixtures/fail/1029-copy-tree-depth.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
}

#[test]
fn projected_exclusive_call_requires_unique_subtree_and_closed_effects() {
    wasm(include_str!("fixtures/pass/1030-copy-exclusive-call-detach.atto"));
    wasm(include_str!("fixtures/pass/1031-copy-exclusive-call-release.atto"));
    assert_eq!(compile(include_str!("fixtures/fail/1032-copy-exclusive-shared-source.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1033-copy-exclusive-shared-copy.atto")).unwrap_err().code, "E_COPY_READONLY");
    assert_eq!(compile(include_str!("fixtures/fail/1034-copy-exclusive-missing-mut.atto")).unwrap_err().code, "E_BORROW");
    assert_eq!(compile(include_str!("fixtures/fail/1035-copy-exclusive-return-escape.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1036-copy-exclusive-nested-effect.atto")).unwrap_err().code, "E_COPY_UNPROVEN");
    assert_eq!(compile(include_str!("fixtures/fail/1037-copy-exclusive-unproved-callee.atto")).unwrap_err().code, "E_BORROW");
}
