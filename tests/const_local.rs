//! Local const is a compile-time provenance requirement, not an alias for val.
fn valid(src: &str) {
    atto_compiler::compile(src).unwrap_or_else(|e| panic!("unexpected compile error: {e}\\n{src}"));
}
fn rejects(src: &str, code: &str) {
    let d = atto_compiler::compile(src).expect_err("expected compile-time rejection");
    assert_eq!(d.code, code, "{d}: {src}");
}

#[test]
fn const_literals_and_derived_constants_are_valid() {
    valid("pub fnc main(): int { const int x = 5; const y: int = x + 2; return y; }");
    valid("pub fnc main(): int { int x = 5; const y = x + 2; return y; }");
    valid("pub fnc main(): int { const x = true; const y = !x; if y then return 0; fi return 1; }");
}

#[test]
fn runtime_initializer_requires_val_or_let() {
    let fnc = "fnc compute(): int { return 7; }";
    rejects(&format!("{fnc} pub fnc main(): int {{ const x = compute(); return x; }}"), "E_CONST");
    rejects(&format!("{fnc} pub fnc main(): int {{ int x = compute(); return x; }}"), "E_CONST");
    valid(&format!("{fnc} pub fnc main(): int {{ val x = compute(); return x; }}"));
    valid(&format!("{fnc} pub fnc main(): int {{ let x = compute(); return x; }}"));
}

#[test]
fn immutable_runtime_binding_is_not_a_const_expression() {
    rejects("pub fnc main(): int { val x = 1; const y = x + 2; return y; }", "E_CONST");
    rejects("pub fnc main(): int { let x = 1; const y = x; return y; }", "E_CONST");
}

#[test]
fn parameters_and_actor_operations_are_not_constant() {
    rejects("fnc identity(int x): int { const y = x; return y; } pub fnc main(): int { return identity(5); }", "E_CONST");
    rejects("pub fnc main(): int { const x = stdio.println(1); return 1; }", "E_CONST");
}

#[test]
fn compile_time_provenance_does_not_grant_rebinding() {
    rejects("pub fnc main(): int { const int x = 5; x = 6; return x; }", "E_IMMUTABLE");
    rejects("pub fnc main(): int { const mut int x = 5; x = 6; return x; }", "E_CONST_MUT_TARGET");
}

#[test]
fn checked_i64_constant_folding_rejects_overflow() {
    rejects("pub fnc main(): int { const x = 9223372036854775807 + 1; return x; }", "E_CONST_OVERFLOW");
    rejects("pub fnc main(): int { const x = -(-9223372036854775808); return x; }", "E_CONST_OVERFLOW");
    rejects("pub fnc main(): int { const x = 3037000500 * 3037000500; return x; }", "E_CONST_OVERFLOW");
}

#[test]
fn negative_i64_min_and_bounded_arithmetic_are_constant() {
    valid("pub fnc main(): int { const int min = -9223372036854775808; const int seven = 2 + 5; return seven; }");
    valid("pub fnc main(): int { const x = 100 - 99; const y = x * 7; return y; }");
}

#[test]
fn folded_booleans_floats_and_strings_have_typed_values() {
    valid(r#"pub fnc main(): int {
        const x = 1.5 + 2.5;
        const y = x * 2.0;
        const is_eight = y eq 8.0;
        const text = "hello" + " world";
        const same = text eq "hello world";
        if is_eight, same then return 1; fi
        return 0;
    }"#);
}

#[test]
fn constant_provenance_follows_slots_not_shadowed_names() {
    valid("pub fnc main(): int { const x = 2; if true then val x = 7; fi const y = x + 3; return y; }");
    rejects("pub fnc main(): int { const x = 2; if true then val x = 7; const y = x; fi return x; }", "E_CONST");
}

#[test]
fn const_cannot_call_functions_or_reach_mutable_aliases() {
    rejects("pub fnc main(): int { let mut x = 1; const y = x; return y; }", "E_CONST");
    rejects("pub fnc main(): int { const x = stdio.println(1); return 0; }", "E_CONST");
}

#[test]
fn type_right_const_and_implicit_const_have_the_same_provenance() {
    valid("pub fnc main(): int { int x = 10; const y: int = 20; const int z = 12; return x + y + z; }");
    rejects("fnc runtime_value(): int { return 10; } pub fnc main(): int { const x: int = runtime_value(); return x; }", "E_CONST");
    rejects("pub fnc main(): int { const x: int = 10; x = 42; return x; }", "E_IMMUTABLE");
}

#[test]
fn bare_mut_is_neither_an_implicit_const_nor_a_binding() {
    rejects("pub fnc main(): int { mut int x = 10; return x; }", "E_MUT_QUALIFIER");
    rejects("pub fnc main(): int { mut Cell x = new Cell(); return 0; }", "E_MUT_QUALIFIER");
}

#[test]
fn const_mut_rejects_immutable_scalar_targets_with_or_without_type_annotations() {
    rejects("pub fnc main(): int { const mut int x = 10; return x; }", "E_CONST_MUT_TARGET");
    rejects("pub fnc main(): int { const mut x: int = 10; return x; }", "E_CONST_MUT_TARGET");
    rejects("pub fnc main(): int { const mut x = 10; return x; }", "E_CONST_MUT_TARGET");
    rejects("pub fnc main(): int { const mut bool x = true; return 0; }", "E_CONST_MUT_TARGET");
    rejects("pub fnc main(): int { const mut string x = \"hello\"; return 0; }", "E_CONST_MUT_TARGET");
}
