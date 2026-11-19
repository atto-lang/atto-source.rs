//! Structural sequence patterns v2: fail-closed compiler admission.
//! These tests intentionally require no guest artifact while the target
 //! language's List/Any/Array/Tuple matcher remains unimplemented.

use atto_compiler::compile;

#[test]
fn structural_sequence_patterns_are_not_misparsed_as_enum_variants() {
    for pattern in [
        "List.of(1, ...)",
        "List.of(..., is int as val x, ...)",
        "Array.of(...(3 of int, 3 of bool))",
        "Tuple.of(...(3 of 7))",
        "List.of(...(2 of List.of(1 as val x)))",
        "List.of(...(2 of List.of(...(1 of int))))",
        "List.of(..._, 1)",
        "List.of(_..., 1)",
    ] {
        let source = format!(
            "pub fnc main(): int {{ return match 1 over on {pattern} -> 42, default -> 0, end }}"
        );
        let err = compile(&source).expect_err("unsupported sequence patterns must emit no wasm");
        assert_eq!(err.code, "E_UNSUPPORTED", "{pattern}: {err}");
        assert!(
            err.message.contains("structural sequence patterns"),
            "{pattern} was incorrectly parsed or rejected at the wrong stage: {err}"
        );
    }
}

#[test]
fn bare_and_counted_rest_outside_a_sequence_fail_closed() {
    for pattern in ["...", "...(3 of int)", "..._", "...(int)..."] {
        let source = format!(
            "pub fnc main(): int {{ return match 1 over on {pattern} -> 42, default -> 0, end }}"
        );
        let err = compile(&source).expect_err("a standalone rest must not produce guest bytes");
        assert_eq!(err.code, "E_UNSUPPORTED", "{pattern}: {err}");
        assert!(err.message.contains("sequence rest/count patterns"), "{pattern}: {err}");
    }
}

#[test]
fn valid_nonstructural_match_retains_aot_wasm_lowering() {
    let source = "pub fnc main(): int { return match 1 over on 1 -> 42, default -> 0, end }";
    let wasm = compile(source).expect("a supported scalar match must still compile");
    assert!(wasm.len() > 8);
    assert_eq!(&wasm[..4], b"\0asm");
}
