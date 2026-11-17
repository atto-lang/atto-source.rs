//! Real Rust/Wasm frontend compilation gates for canonical copy!! syntax.
//! Scalar guest Wasm can execute; heap graphs MUST fail closed until the
//! generated AOT copy plan, Sendable proof and allocator are implemented.
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
fn copy_deep_cannot_lower_heap_handles_as_shallow_copies() {
    for source in [
        "define class Parcel as pub let int value = 1; end pub fnc main(): int { val p = new Parcel(); val q = copy!! p; return 1; }",
        "define class Parcel as pub let int value = 1; end pub fnc main(): int { val q = copy!! (new Parcel()); return 1; }",
    ] {
        let diagnostic = compile(source).expect_err("no generated object deep-copier yet");
        assert_eq!(diagnostic.code, "E_COPY_DEEP_UNSUPPORTED", "{diagnostic}");
    }
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
    // Shallow reference copying remains rejected with the existing code;
    // deep reference copying is a distinct, explicit error.
    let prefix = "define class Parcel as pub let int value = 1; end pub fnc main(): int { val p = new Parcel(); ";
    for (expr, code) in [
        ("copy! p", "E_COPY"),
        ("copy!! p", "E_COPY_DEEP_UNSUPPORTED"),
    ] {
        let source = format!("{prefix}val q = {expr}; return 0; }}");
        assert_eq!(compile(&source).unwrap_err().code, code);
    }
}
