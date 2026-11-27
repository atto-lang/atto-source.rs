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

#[test]
fn explicit_counter_and_compound_updates() {
    let strict = atto_compiler::compile_explicit_rebinding;
        let positives = [
            "pub fnc main(): int { let x = 40; set x++; set ++x; return x; }",
            "pub fnc main(): int { let x = 44; set x--; set --x; return x; }",
            "pub fnc main(): int { let x = 20; set x += 2; set x -= 1; set x *= 2; return x; }",
            "pub fnc main(): int { let x = 32; for let i = 0; i < 10; set i++ { set x++; } return x; }",
            "pub fnc main(): int { let x = 32; for (let i = 10; i > 0; set --i) { set x++; } return x; }",
            "pub fnc main(): int { let x = 32; for let i = 0; i < 10; set i += 2 { set x += 2; } return x; }",
        ];
        let negatives = [
            "pub fnc main(): int { let x = 40; x++; return x; }",
            "pub fnc main(): int { let x = 40; x += 2; return x; }",
            "pub fnc main(): int { let x = 40; ++x; return x; }",
            "pub fnc main(): int { for let i = 0; i < 10; i++ {} return 42; }",
            "pub fnc main(): int { for let i = 0; i < 10; let i++ {} return 42; }",
            "pub fnc main(): int { const x = 40; set x++; return x; }",
            "pub fnc main(): int { set ++x; return 42; }",
            "pub fnc main(): int { let x = true; set x++; return 42; }",
            "pub fnc main(): int { let x = 40; set x - = 2; return x; }",
            "pub fnc main(): int { let x = 40; set x * /* gap */ = 2; return x; }",
            "pub fnc main(): int { let x = 40; set ++x++; return x; }",
        ];
        for source in positives { strict(source).unwrap_or_else(|e| panic!("{source}: {e}")); }
        for source in negatives { assert!(strict(source).is_err(), "{source}"); }
}

#[test]
fn scalar_match_aliases_are_fixed_unless_rebind_is_explicit() {
    valid("pub fnc main(): int { return match 42 over on _ as int z -> z end; }");
    valid("pub fnc main(): int { return match 42 over on _ as const z -> z end; }");
    valid("pub fnc main(): int { let int n = 20; let int r = 0; do match n over on _ as rebind int z -> { z = z + 2; r = z; } end return n + r; }");
    valid("pub fnc main(): int { return match 42 over on 42 as rebind z when z eq 42 -> { z = z + 1; return z; } on _ -> 0 end; }");
    valid("pub fnc main(): int { return match 42 over on 41 as z when z eq 42 -> 0, on _ as const z -> z end; }");
    reject("pub fnc main(): int { do match 42 over on _ as int z -> { z = 0; } end return 0; }", "E_IMMUTABLE");
    reject("pub fnc main(): int { return match 42 over on _ as bool z -> 0 end; }", "E_TYPE");
    reject("pub fnc main(): int { return match 42 over on _ as rebind rebind int z -> z end; }", "E_SYNTAX");
    reject("pub fnc main(): int { return match 42 over on _ as mut z -> 0 end; }", "E_UNSUPPORTED");
    reject("pub fnc main(): int { do match 42 over on _ as rebind z -> {} end return z; }", "E_UNBOUND");
}
