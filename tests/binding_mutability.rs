//! Contract for independent rebinding and owned-referent field mutation.
//! The browser subset has owned class locals, not unrestricted references.

const CLASS: &str = r#"
define class Cell as
    pub let int value = 1;
    pub final int fixed = 9;
end
"#;

fn program(body: &str) -> String {
    format!("{CLASS}\\npub fnc main(): int {{\\n{body}\\n}}")
}

fn expect_ok(source: &str) {
    atto_compiler::compile(source).unwrap_or_else(|e| panic!("unexpected error: {e}\\n{source}"));
}

fn expect_error(body: &str, code: &str) {
    let source = program(body);
    let err = atto_compiler::compile(&source).expect_err("expected compiler rejection");
    assert_eq!(err.code, code, "{err}: {source}");
}

#[test]
fn all_four_binding_modes_have_independent_capabilities() {
    expect_ok(&program(r#"
        val Cell immutable = new Cell();
        val mut Cell fixed_name = new Cell();
        fixed_name.value = 3;
        let Cell replaceable = new Cell();
        replaceable = new Cell();
        let mut Cell mutable = new Cell();
        mutable.value = 5;
        mutable = new Cell();
        mutable.value = 7;
        return immutable.value + fixed_name.value + replaceable.value + mutable.value;
    "#));
}

#[test]
fn val_and_let_without_mut_cannot_write_fields() {
    expect_error("val Cell v = new Cell(); v.value = 2; return v.value;", "E_IMMUTABLE");
    expect_error("let Cell v = new Cell(); v.value = 2; return v.value;", "E_IMMUTABLE");
}

#[test]
fn val_and_val_mut_names_cannot_be_rebound() {
    expect_error("val Cell v = new Cell(); v = new Cell(); return v.value;", "E_IMMUTABLE");
    expect_error("val mut Cell v = new Cell(); v = new Cell(); return v.value;", "E_IMMUTABLE");
}

#[test]
fn mut_does_not_override_final_fields_or_ownership() {
    expect_error("let mut Cell v = new Cell(); v.fixed = 4; return 0;", "E_IMMUTABLE");
    expect_error("val mut Cell v = new Cell(); val Cell moved = v; v.value = 2; return 0;", "E_MOVED");
}

#[test]
fn scalars_still_distinguish_reassignment_from_mut() {
    expect_ok("pub fnc main(): int { let x: int = 1; x = 2; let mut y: int = 3; y = 4; return x + y; }");
    let src = "pub fnc main(): int { val mut x: int = 1; x = 2; return x; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_IMMUTABLE");
}

#[test]
fn legacy_const_mut_is_still_not_rebindable() {
    expect_ok("pub fnc main(): int { const mut x: int = 1; return x; }");
    let src = "pub fnc main(): int { const mut x: int = 1; x = 2; return x; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_IMMUTABLE");
}
