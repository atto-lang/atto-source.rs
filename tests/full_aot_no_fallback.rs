//! Rust/Wasm compiler AOT baseline, not a claim of native LLVM/Java/.NET parity.
//! A recognized feature must become an actual binary guest or return a diagnostic.
//! No dynamic source evaluator, runtime compilation or implicit interpreter fallback.
use atto_compiler::compile;

const WASM_V1: &[u8] = b"\0asm\x01\0\0\0";

fn read_uleb(bytes: &[u8], cursor: &mut usize) -> u32 {
    let mut value = 0u32;
    for shift in [0, 7, 14, 21, 28] {
        let octet = *bytes.get(*cursor).expect("truncated Wasm u32 LEB");
        *cursor += 1;
        if shift == 28 {
            assert!(octet <= 0x0f, "overlong/overflowing Wasm u32");
        }
        value |= ((octet & 0x7f) as u32) << shift;
        if octet & 0x80 == 0 {
            return value;
        }
    }
    panic!("overlong Wasm LEB");
}

fn wasm_sections(bytes: &[u8]) -> Vec<(u8, &[u8])> {
    assert!(bytes.starts_with(WASM_V1), "must emit Wasm v1 binary");
    let mut cursor = WASM_V1.len();
    let mut sections = Vec::new();
    while cursor < bytes.len() {
        let id = bytes[cursor];
        cursor += 1;
        let len = read_uleb(bytes, &mut cursor) as usize;
        let end = cursor.checked_add(len).expect("Wasm section length overflow");
        assert!(end <= bytes.len(), "Wasm section exceeds binary");
        sections.push((id, &bytes[cursor..end]));
        cursor = end;
    }
    sections
}

#[test]
fn admitted_closed_world_features_emit_deterministic_executable_code_sections() {
    for (feature, source) in [
        ("scalar-direct-call", include_str!("fixtures/pass/01-answer.atto")),
        ("recursive-call", include_str!("fixtures/pass/19-recursion-fib.atto")),
        ("closed-lexical-closure", include_str!("fixtures/pass/lambda-direct-int.atto")),
        ("tagged-option-match", include_str!("fixtures/pass/85-option-some-match.atto")),
    ] {
        let wasm = compile(source).unwrap_or_else(|e| panic!("{feature} failed AOT compilation: {e}"));
        assert_eq!(wasm, compile(source).expect("same source should compile"), "{feature}: deterministic binary");
        let sections = wasm_sections(&wasm);
        let code = sections.iter().find(|(id, _)| *id == 10).expect("real Wasm code section");
        let mut pos = 0;
        assert!(read_uleb(code.1, &mut pos) > 0, "{feature}: no compiled function bodies");
        // These pure fixtures never call host facilities. Adding a source
        // evaluator/compile service as an import would violate their AOT ABI.
        let imports = sections.iter().find(|(id, _)| *id == 2);
        if let Some((_, payload)) = imports {
            let mut pos = 0;
            assert_eq!(read_uleb(payload, &mut pos), 0, "{feature}: unexpected host import");
        }
    }
}

#[test]
fn runtime_source_compilation_and_arbitrary_reflection_do_not_fall_back() {
    for (feature, source) in [
        ("runtime-eval", r#"pub fnc main(): int { return eval("40 + 2"); }"#),
        ("runtime-compiler", r#"pub fnc main(): int { return compile_source("40 + 2"); }"#),
        ("runtime-reflection", r#"pub fnc main(): int { return reflect_call(42, "add", 1); }"#),
        ("unlinked-native-dynamic", r#"pub fnc main(): int { return load_native_symbol("lib", "f"); }"#),
        ("runtime-type-creation", r#"pub fnc main(): int { return new_type("Unknown"); }"#),
    ] {
        let diagnostic = compile(source).expect_err("unverified feature must emit no guest artifact");
        assert!(!diagnostic.code.is_empty(), "{feature}: require diagnostic code");
        assert_eq!(
            diagnostic.code,
            compile(source).expect_err("retry must also reject").code,
            "{feature}: diagnostic stability"
        );
    }
}

#[test]
fn references_to_dynamic_keywords_in_comments_do_not_switch_compilation_modes() {
    let source = r#"
        // eval("source"); compile_source("source"); reflect_call(...);
        pub fnc main(): int { return 42; }
    "#;
    let aot = compile(source).expect("lexical comments must not be interpreted as code");
    assert!(aot.starts_with(WASM_V1));
    assert!(wasm_sections(&aot).iter().any(|(id, _)| *id == 10));
}
