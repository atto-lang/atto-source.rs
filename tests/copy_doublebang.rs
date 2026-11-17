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
fn copy_deep_disallows_mut_qualifier_and_missing_operand() {
    let diagnostic = compile("pub fnc main(): int { val x = 1; return copy!! mut x; }")
        .expect_err("mut qualifier is reserved for borrow!");
    assert_eq!(diagnostic.code, "E_SYNTAX");
    assert!(compile("pub fnc main(): int { return copy!!; }").is_err());
}

#[test]
fn separate_bangs_and_spelling_have_distinct_parsed_behavior() {
    // A shallow class copy without verified Copyable witness must still
    // fail. The deep path is independently admitted for this flat class.
    let prefix = "define class Parcel as pub let int value = 1; end pub fnc main(): int { val p = new Parcel(); ";
    let shallow = format!("{prefix}val q = copy! p; return 0; }}");
    assert_eq!(compile(&shallow).unwrap_err().code, "E_COPY");
    let deep = format!("{prefix}val q = copy!! p; return q.value; }}");
    wasm(&deep);
}
