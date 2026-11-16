//! Contract for independent rebinding and owned-referent field mutation.
//! The browser subset has owned class locals, not unrestricted references.

const CLASS: &str = r#"
define class Cell as
    pub let int value = 1;
    pub final int fixed = 9;
end
"#;

fn program(body: &str) -> String {
    format!("{CLASS}\npub fnc main(): int {{\n{body}\n}}")
}

fn expect_ok(source: &str) {
    atto_compiler::compile(source).unwrap_or_else(|e| panic!("unexpected error: {e}\n{source}"));
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
fn scalar_const_mut_is_not_a_mutable_binding() {
    expect_error("const mut x: int = 1; return x;", "E_CONST_MUT_TARGET");
    expect_error("const mut int x = 1; x = 2; return x;", "E_CONST_MUT_TARGET");
}

#[test]
fn mutable_binding_does_not_override_private_visibility() {
    let source = r#"
        define class Secret as
            let int private_value = 1;
        end
        pub fnc main(): int {
            let mut Secret x = new Secret();
            x.private_value = 4;
            return 0;
        }
    "#;
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_PRIVATE");
}

#[test]
fn mutable_binding_does_not_enable_unknown_or_nested_lvalues() {
    expect_error("let mut Cell x = new Cell(); x.no_field = 3; return 0;", "E_UNBOUND");
    // Nested field targets are rejected before any unsafe lvalue lowering.
    // A scalar member is enough to exercise the nested target grammar.
    expect_error("let mut Cell x = new Cell(); x.value.any = 3; return 0;", "E_OWNERSHIP");
}

#[test]
fn four_modes_support_name_first_typed_annotations() {
    expect_ok(&program(r#"
        val x: Cell = new Cell();
        val mut y: Cell = new Cell();
        y.value = 2;
        let z: Cell = new Cell();
        z = new Cell();
        let mut w: Cell = new Cell();
        w.value = 3;
        return x.value + y.value + z.value + w.value;
    "#));
}

#[test]
fn mutable_local_cannot_use_an_object_after_take() {
    expect_error("let mut Cell x = new Cell(); val Cell taken = x; x.value = 3; return 0;", "E_MOVED");
    expect_error("val mut Cell x = new Cell(); val Cell taken = x; x.value = 3; return 0;", "E_MOVED");
}

#[test]
fn mutable_referent_and_rebinding_are_checked_on_each_operation() {
    expect_error("let Cell x = new Cell(); x = new Cell(); x.value = 3; return 0;", "E_IMMUTABLE");
    expect_error("val mut Cell x = new Cell(); x.value = 3; x = new Cell(); return 0;", "E_IMMUTABLE");
}
