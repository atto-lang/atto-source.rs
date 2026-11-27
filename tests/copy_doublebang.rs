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
