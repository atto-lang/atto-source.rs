//! End-to-end browser compiler admission for bounded scalar generics.
//! These cases must run against a real Rust parser, checked IR and Wasm emitter.

use atto_compiler::compile;

#[test]
fn contextual_and_explicit_scalar_generic_calls_compile() {
    for src in [
        "fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate(5) as float; if x eq 8.0 then return 42; fi return 0; }",
        "fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate(5) : int; return x; }",
        "fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x: float = calculate(5); if x eq 8.0 then return 42; fi return 0; }",
        "fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate<float>(5); if x eq 8.0 then return 42; fi return 0; }",
        "fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate<int>(5); return x; }",
        "fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val a = calculate<int>(5); val b = calculate<float>(5); if b eq 8.0 then return a; fi return 0; }",
    ] {
        let bytes = compile(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    }
}

#[test]
fn unsupported_or_conflicting_generic_inference_fails_before_wasm_emission() {
    for (source, expected) in [
        ("fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate(5); return 0; }", "E_INFER"),
        ("fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate<float>(5) : int; return 0; }", "E_TYPE"),
        // The existing nominal Array<T> source preflight rejects before the
        // bounded scalar generic specializer: rejection remains fail-closed.
        ("fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate<Array<int>>(5); return 0; }", "E_UNSUPPORTED"),
        ("fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { return 0; }", "E_UNSUPPORTED"),
        ("fnc calculate<T>(int x): T { return (x + 3) as T; } pub fnc main(): int { val x = calculate<bool>(5); return 0; }", "E_UNSUPPORTED_CAST"),
    ] {
        let error = compile(source).expect_err(source);
        assert_eq!(error.code, expected, "{source}: {error}");
    }
}
