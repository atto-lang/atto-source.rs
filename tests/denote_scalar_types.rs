//! Closed scalar-denotation candidate: actual compiler output, not a text rewrite.
use atto_compiler::{compile, lexer, parser::Parser};

#[test]
fn transparent_aliases_compile_to_identical_wasm() {
    let original = "pub fnc main(): int { const int n = 42; return n; }";
    let denoted = "denote type Count = int; denote type Total = Count; pub fnc main(): Total { const Count n = 42; return n; }";
    let expected = compile(original).expect("primitive control");
    let actual = compile(denoted).expect("transparent scalar denotation");
    assert!(actual.starts_with(b"\0asm"));
    assert_eq!(expected, actual);
}

#[test]
fn checker_does_not_erase_scalar_type_mismatch() {
    assert!(compile("denote type Count = int; pub fnc main(): int { const Count n = true; return n; }").is_err());
}

#[test]
fn unsupported_and_invalid_type_declarations_fail_closed() {
    for decl in [
        "denote type Count = 5 | 6 | 7;",
        "denote type Count = Count;",
        "denote type Count = Missing;",
        "denote type Count = int; denote type Count = bool;",
        "denote type String = int;",
        "denote type Array = int;",
        "denote type Count = int",
        "denote pub type Count = int;",
        "denote type Count<T> = T;",
        "denote type Count = int + int;",
        "denote actor Count;",
    ] {
        let src = format!("{decl} pub fnc main(): int {{ return 42; }}");
        assert!(compile(&src).is_err(), "{src}");
    }
}

#[test]
fn nominal_names_cannot_be_rebound_as_scalars() {
    let src = "denote type Count = int; define enum Count as A end pub fnc main(): int { return 42; }";
    let error = Parser::new(lexer::lex(src).unwrap()).parse_program().expect_err("nominal collision");
    assert_eq!(error.code, "E_DUPLICATE");
}

#[test]
fn contextual_word_does_not_reserve_callable_name() {
    assert!(compile("fnc denote(int n): int { return n; } pub fnc main(): int { return denote(42); }").is_ok());
}
