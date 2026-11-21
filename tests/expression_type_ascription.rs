//! Type ascription and checked-as browser-compiler admission.
//! This is a monomorphic/contextual-ADT milestone, not generic specialization.

use atto_compiler::compile;

fn source(expr: &str) -> String {
    format!("pub fnc main(): int {{ return {expr}; }}")
}

#[test]
fn scalar_ascription_and_identity_cast_emit_wasm() {
    for expr in [
        "42 : int",
        "42 as int",
        "(40 + 2) : int",
        "(40 + 2) as int",
        "true ? 42 : 7",
        "true ? (42 : int) : 7",
        "(true ? 42 : 7) : int",
        "(true ? 42 : 7) as int",
    ] {
        let bytes = compile(&source(expr)).unwrap_or_else(|e| panic!("{expr}: {e}"));
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0", "{expr}");
    }
}

#[test]
fn nested_ascriptions_preserve_contextual_adt_construction() {
    for annotation in [": Result<int,string>", "as Result<int,string>"] {
        let source = format!(
            "fnc make(): Result<int,string> {{ return Ok(42) {annotation}; }} \
             pub fnc main(): int {{ return 42; }}"
        );
        let bytes = compile(&source).unwrap_or_else(|e| panic!("{annotation}: {e}"));
        assert!(bytes.starts_with(b"\0asm"), "{annotation}");
    }
}

#[test]
fn mismatched_ascriptions_and_unlowered_casts_fail_closed() {
    for (expr, expected) in [
        ("42 : bool", "E_TYPE"),
        ("42 as bool", "E_UNSUPPORTED_CAST"),
        ("42 : float", "E_TYPE"),
        ("42.0 as int", "E_UNSUPPORTED_CAST"),
        ("42 : void", "E_TYPE"),
    ] {
        let err = compile(&source(expr)).unwrap_err();
        assert_eq!(err.code, expected, "{expr}: {err}");
    }
}

#[test]
fn explicit_int_to_float_conversion_compiles_to_real_wasm() {
    for src in [
        "pub fnc main(): int { val converted = 42 as float; if converted eq 42.0 then return 42; fi return 0; }",
        "pub fnc main(): int { val converted = (40 + 2) as float; if converted eq 42.0 then return 42; fi return 0; }",
    ] {
        let bytes = compile(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(bytes.starts_with(b"\0asm"));
    }
}

#[test]
fn ternary_separator_remains_distinct_from_type_ascription() {
    let yes = compile(&source("true ? 42 : 7")).unwrap();
    let no = compile(&source("false ? 42 : 7")).unwrap();
    assert!(yes.starts_with(b"\0asm"));
    assert!(no.starts_with(b"\0asm"));
    assert_eq!(compile(&source("true ? 42 : bool")).unwrap_err().code, "E_UNBOUND");
}

#[test]
fn generic_calls_and_value_match_select_do_not_publish_fake_guests() {
    // These are language-design targets, not yet executable in the
    // bounded browser Rust/Wasm frontend. No syntax normalization may
    // accidentally turn them into something with unrelated semantics.
    for src in [
        "pub fnc main(): int { return calculate<int>(5) as int; }",
        "pub fnc main(): int { return (match status over on _ -> 42 end) : int; }",
        "pub fnc main(): int { return (select over when readch ch -> 42 end) as int; }",
    ] {
        let err = compile(src).expect_err("unsupported browser construct must fail closed");
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}: {err}");
    }
}
