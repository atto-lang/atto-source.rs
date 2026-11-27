//! Scalar-only rebind admission; reference ownership is not implemented.
use atto_compiler::compile;

fn valid(source: &str) {
    let bytes = compile(source).expect("scalar rebind should compile");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
}

fn reject(source: &str, code: &str) {
    let error = compile(source).expect_err("rejected source must emit no guest");
    assert_eq!(error.code, code, "{error}");
}

#[test]
fn scalar_rebind_type_first_and_name_first() {
    valid("fnc bump(rebind int n): int { n = n + 1; return n; } pub fnc main(): int { return bump(41); }");
    valid("fnc bump(rebind n: int): int { n = n + 1; return n; } pub fnc main(): int { return bump(41); }");
}

#[test]
fn fixed_scalar_and_contextual_identifier() {
    reject("fnc bump(int n): int { n = n + 1; return n; } pub fnc main(): int { return bump(41); }", "E_IMMUTABLE");
    valid("fnc id(int rebind): int { return rebind; } pub fnc main(): int { return id(42); }");
    valid("fnc id(rebind: int): int { return rebind; } pub fnc main(): int { return id(42); }");
}

#[test]
fn reject_unimplemented_reference_replacement_and_duplicate_marker() {
    reject("fnc replace(rebind mut Foo f): void {} pub fnc main(): int { return 0; }", "E_UNSUPPORTED");
    reject("fnc bump(rebind rebind int n): int { return n; } pub fnc main(): int { return bump(41); }", "E_SYNTAX");
    reject("fnc replace(rebind Foo f): void {} pub fnc main(): int { return 0; }", "E_UNSUPPORTED");
}

#[test]
fn unused_marker_emits_identical_wasm() {
    let fixed = "fnc id(int n): int { return n; } pub fnc main(): int { return id(42); }";
    let marked = fixed.replace("int n", "rebind int n");
    assert_eq!(compile(fixed).unwrap(), compile(&marked).unwrap(),
        "unused rebinding permission must not change emitted Wasm");
}

#[test]
fn explicit_scalar_set_and_fresh_let_shadow() {
    for source in [
        "pub fnc main(): int { let int x = 40; set x = x + 2; return x; }",
        "pub fnc main(): int { let int x = 40; set! x = x + 2; return x; }",
        "pub fnc main(): int { let int x = 40; set! (x = x + 2); return x; }",
        "pub fnc main(): int { let x = 40; let x = x + 1; set x = x + 1; return x; }",
        "pub fnc main(): int { let x = 40; let x = true; if x then return 42; else return 0; fi }",
    ] {
        valid(source);
    }
}

#[test]
fn explicit_set_respects_fixed_and_nearest_binding() {
    reject("pub fnc main(): int { const x = 40; set x = 42; return x; }", "E_IMMUTABLE");
    reject("pub fnc main(): int { val int x = 40; set! x = 42; return x; }", "E_IMMUTABLE");
    reject("pub fnc main(): int { set x = 42; return 0; }", "E_UNBOUND");
    reject("pub fnc main(): int { const x = 1; const x = 2; return x; }", "E_DUPLICATE");
    reject("pub fnc main(): int { const x = 1; let x = 2; return x; }", "E_DUPLICATE");
    reject("pub fnc main(): int { let x = 1; const x = 2; return x; }", "E_DUPLICATE");
    for source in [
        "pub fnc main(): int { let x = 40; (set! x) = 42; return x; }",
        "pub fnc main(): int { let x = 40; set x.field = 42; return x; }",
        "pub fnc main(): int { let x = 1; let x = true; set x = 2; return 42; }",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn explicit_binding_preview_checks_original_source() {
    use atto_compiler::compile_explicit_rebinding;
    let old = "pub fnc main(): int { let x = 40; x = x + 2; return x; }";
    let explicit = old.replace("x = x + 2", "set x = x + 2");
    let expected = compile(old).unwrap();
    assert_eq!(compile_explicit_rebinding(&explicit).unwrap(), expected);
    for source in [
        old,
        "pub fnc main(): int { let x = 0; for (x = 1; x < 2; x++) {} return x; }",
        "pub fnc main(): int { let x = 0; for (; x < 2; x = x + 1) {} return x; }",
    ] {
        assert_eq!(compile_explicit_rebinding(source).unwrap_err().code,
            "E_EXPLICIT_REBIND", "{source}");
    }
    let source = "pub fnc main(): int { let x = 0; for (set x = 40; x < 42; set! (x = x + 1)) {} return x; }";
    assert!(compile_explicit_rebinding(source).is_ok());
    let fixed = "pub fnc main(): int { const x = 40; set x = 42; return x; }";
    assert_eq!(compile_explicit_rebinding(fixed).unwrap_err().code,"E_IMMUTABLE");
}
