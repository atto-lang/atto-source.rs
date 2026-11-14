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
    rejects("pub fnc main(): int { const mut int x = 5; x = 6; return x; }", "E_IMMUTABLE");
}
