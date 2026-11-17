//! Closed scalar Record bracket/dot typing and escaped-name regression gates.
//! Compiling a guest artifact is not substituted for actual execution:
//! tests/closed_record_guest.mjs exercises native and browser-compiled Wasm.

use atto_compiler::{compile, lexer::{self, Tok, MAX_STRING_BYTES}};

fn admit(source: &str) {
    let wasm = compile(source).unwrap_or_else(|d| panic!("{source}\n{d}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "valid WebAssembly header");
    assert_eq!(wasm, compile(source).unwrap(), "deterministic AOT output");
}

fn reject(source: &str, code: &str) {
    let error = compile(source).expect_err("invalid code must not emit guest bytes");
    assert_eq!(error.code, code, "{source}\n{error}");
}

#[test]
fn dot_and_bracket_have_identical_field_types_and_work_in_an_expression() {
    admit(r#"
        fnc double(value: int): int { return value + value; }
        pub fnc main(): int {
            let Record headers = {status_code: 21, "content-type": "application/json"};
            let int a = headers.status_code;
            let int b = headers["status_code"];
            let string content = headers["content-type"];
            stdio.println(content);
            return double(a) + b - 21;
        }
    "#);
}

#[test]
fn bracket_keys_decode_escaped_quotes_backslashes_tabs_and_unicode() {
    admit(r#"
        pub fnc main(): int {
            let Record headers = {
                "content\"type": 3,
                "a\\b": 4,
                "tab\tfield": 5,
                "uni\u{63}ode": 30
            };
            return headers["content\"type"] +
                headers["a\\b"] +
                headers["tab\tfield"] +
                headers["unicode"];
        }
    "#);
}

#[test]
fn escaped_and_literal_names_are_the_same_structural_field() {
    admit(r#"
        pub fnc main(): int {
            let Record headers = {"status_code": 21, "\u{63}ontent-type": 21};
            return headers.status_code + headers["content-type"];
        }
    "#);
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": 21, "\u{63}ontent-type": 21};
            return 0;
        }
    "#, "E_DUPLICATE");
}

#[test]
fn static_bracket_typing_checks_annotations_instead_of_erasing_types() {
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": "json", status_code: 42};
            let int invalid = headers["content-type"];
            return invalid;
        }
    "#, "E_TYPE");
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": "json", status_code: 42};
            let string invalid = headers.status_code;
            return 0;
        }
    "#, "E_TYPE");
}

#[test]
fn absent_record_fields_are_precise_across_both_access_forms() {
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": "json", status_code: 42};
            return headers.no_such_field;
        }
    "#, "E_NO_FIELD");
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": "json", status_code: 42};
            return headers["no-such-field"];
        }
    "#, "E_NO_FIELD");
}

#[test]
fn dynamic_key_and_opaque_nested_record_layouts_are_not_unsafely_converted() {
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": 42};
            let string key = "content-type";
            return headers[key];
        }
    "#, "E_UNSUPPORTED");
    reject(r#"
        pub fnc main(): int {
            let Record headers = {nested: {status_code: 42}};
            return headers.nested["status_code"];
        }
    "#, "E_UNSUPPORTED");
}

#[test]
fn lexical_shadows_cannot_inherit_record_field_permissions() {
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"content-type": 42};
            block {
                let int headers = 7;
                stdio.println(headers["content-type"]);
            }
            return 0;
        }
    "#, "E_UNSUPPORTED");
}

#[test]
fn decoded_escape_strings_respect_utf8_byte_budget() {
    let escapes = r"\u{61}".repeat(MAX_STRING_BYTES + 1);
    let string = format!("\"{escapes}\"");
    let err = lexer::lex(&string).expect_err("escapes must not bypass decoded-string limit");
    assert_eq!(err.code, "E_SOURCE_LIMIT");

    let record = format!(
        "pub fnc main(): int {{ let Record h = {{\"{escapes}\": 42}}; return 0; }}"
    );
    reject(&record, "E_SOURCE_LIMIT");
}

#[test]
fn exact_utf8_budget_allows_valid_strings_but_rejects_one_extra_escape() {
    let allowed = r"\u{61}".repeat(MAX_STRING_BYTES);
    let tokens = lexer::lex(&format!("\"{allowed}\"")).unwrap();
    assert!(matches!(&tokens[0].tok, Tok::Str(s) if s.len() == MAX_STRING_BYTES));

    let extra = format!("{}{}", allowed, r"\u{61}");
    assert_eq!(lexer::lex(&format!("\"{extra}\"")).unwrap_err().code, "E_SOURCE_LIMIT");

    let multibyte = r"\u{E9}".repeat(MAX_STRING_BYTES / 2 + 1);
    assert_eq!(
        lexer::lex(&format!("\"{multibyte}\"")).unwrap_err().code,
        "E_SOURCE_LIMIT",
        "budget is UTF-8 bytes, not Unicode codepoint count"
    );
}

#[test]
fn invalid_escaped_unicode_scalar_fails_at_lexing_not_as_field_lookup() {
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"\u{D800}": 42};
            return headers["\u{D800}"];
        }
    "#, "E_STRING");
    reject(r#"
        pub fnc main(): int {
            let Record headers = {"\u{}": 42};
            return headers["\u{}"];
        }
    "#, "E_STRING");
}
