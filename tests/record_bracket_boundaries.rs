//! Additional executable closed Record boundary/security regression tests.
//! The guest runner in closed_record_guest.mjs verifies native/browser Wasm
//! behavior. These host tests also insist on deterministic admitted bytes and
//! precise fail-closed errors with no generated artifact.

use atto_compiler::{
    closed_record_guest, collection_types, compile, lexer, parser::Parser,
};

fn program(body: &str) -> String {
    format!("pub fnc main(): int {{\n{body}\n}}")
}

fn admit(body: &str) {
    let source = program(body);
    let first = compile(&source).unwrap_or_else(|error| panic!("{source}\n{error}"));
    assert!(first.starts_with(b"\0asm\x01\0\0\0"), "real Wasm guest required");
    assert_eq!(first, compile(&source).unwrap(), "compiler must be deterministic");
}

fn reject(body: &str, code: &str) {
    let source = program(body);
    let error = compile(&source).expect_err("invalid Record program must not emit Wasm");
    assert_eq!(error.code, code, "{source}\n{error}");
}

#[test]
fn eight_static_fields_are_executable_in_canonical_slots() {
    admit(
        r#"let Record r = {h: 35, g: 2, f: 3, e: 4, d: 5, c: 6, b: 8, a: 7};
           return r["a"] + r.h;"#,
    );
}

#[test]
fn ninth_field_rejects_before_guest_allocation() {
    reject(
        r#"let Record r = {a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9};
           return r["a"];"#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn empty_closed_record_does_not_fabricate_a_guest_shape() {
    reject("let Record r = {}; return 0;", "E_UNSUPPORTED");
}

#[test]
fn exactly_thirty_two_shapes_are_accepted_but_thirty_three_reject() {
    let mut body = String::new();
    for i in 0..32 {
        body.push_str(&format!("let Record r{i} = {{f{i}: {i}}};\n"));
    }
    body.push_str("return r31.f31 + 11;");
    admit(&body);

    let mut overflow = String::new();
    for i in 0..33 {
        overflow.push_str(&format!("let Record r{i} = {{f{i}: {i}}};\n"));
    }
    overflow.push_str("return r32.f32 + 10;");
    reject(&overflow, "E_SHAPE");
}

#[test]
fn reused_shape_does_not_consume_another_shape_budget_slot() {
    let mut body = String::new();
    for i in 0..32 {
        body.push_str(&format!("let Record r{i} = {{f{i}: {i}}};\n"));
    }
    body.push_str("let Record again = {f0: 42}; return again.f0;");
    admit(&body);
}

#[test]
fn same_field_name_with_different_scalar_types_interns_separately() {
    let source = program(
        r#"let Record one = {value: 42};
           let Record two = {value: true};
           two.value;
           return one["value"];"#,
    );
    let parsed = Parser::new(lexer::lex(&source).unwrap())
        .parse_program()
        .unwrap();
    let checked = collection_types::analyze(&parsed).unwrap();
    let mut lowered = parsed.clone();
    closed_record_guest::lower(&mut lowered, &checked).unwrap();
    assert_eq!(lowered.classes.len(), parsed.classes.len() + 2);
    admit(
        r#"let Record one = {value: 42};
           let Record two = {value: true};
           two.value;
           return one["value"];"#,
    );
}

#[test]
fn mixed_scalar_field_types_do_not_cross_read_offsets() {
    admit(
        r#"let Record r = {f: 1.5, b: true, s: "ok", i: 42};
           r["f"];
           r["b"];
           r["s"];
           return r["i"];"#,
    );
    reject(
        r#"let Record r = {i: 42, s: "ok"};
           let int wrong = r["s"];
           return wrong;"#,
        "E_TYPE",
    );
}

#[test]
fn exact_quoted_punctuation_and_multibyte_names_are_static_fields() {
    admit(
        r#"let Record r = {"🚀 launch": 40, "x.y:z": 2};
           return r["🚀 launch"] + r["x.y:z"];"#,
    );
    reject(
        r#"let Record r = {"🚀 launch": 42};
           return r["🚀 Launch"];"#,
        "E_NO_FIELD",
    );
}

#[test]
fn expression_keys_are_never_promoted_to_literal_static_fields() {
    reject(
        r#"let Record r = {"content-type": 42};
           return r["content" + "-type"];"#,
        "E_UNSUPPORTED",
    );
    reject(
        r#"let Record r = {x: 42};
           let string key = "x";
           return r[key];"#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn scalar_indexing_cannot_borrow_a_record_field_capability() {
    reject(
        r#"let Record r = {x: 42};
           let int r2 = 7;
           return r2["x"];"#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn ordinary_string_map_index_is_not_a_closed_record_field() {
    reject(
        r#"let Map<string, int> m = {"x" => 42};
           return m["x"];"#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn immutable_aliases_retain_checked_quoted_field_identity() {
    admit(
        r#"let Record r = {"": 41, "x-y": 1};
           val copy = r;
           return copy[""] + copy["x-y"];"#,
    );
}
