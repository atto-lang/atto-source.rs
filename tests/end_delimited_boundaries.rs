//! End-delimited callable boundaries must agree with the Java lexer.
//! This deliberately exercises real parser and Wasm lowering, not text matching.

use atto_compiler::{compile, lexer::{lex, Tok}, parser::Parser};

const SOURCE: &str = "fnc add_one(int x) -> int\n  return x + 1;\nend\npub fnc main(): int { return add_one(41); }";

#[test]
fn lf_crlf_and_bare_cr_have_identical_executable_semantics() {
    let expected = compile(SOURCE).expect("LF source compiles");
    assert!(expected.starts_with(b"\\0asm\\x01\\0\\0\\0"));
    for sep in ["\n", "\r\n", "\r"] {
        let source = SOURCE.replace('\n', sep);
        let actual = compile(&source).unwrap_or_else(|d| panic!("{sep:?}: {d}"));
        assert_eq!(actual, expected, "{sep:?}: line endings must not change guest Wasm");
        let tokens = lex(&source).unwrap();
        let closing = tokens.iter().find(|t| t.tok == Tok::Ident("end".into())).unwrap();
        assert_eq!((closing.pos.line, closing.pos.col), (3, 1), "{sep:?}");
    }
}

#[test]
fn line_comments_stop_at_lf_crlf_or_bare_cr() {
    for sep in ["\n", "\r\n", "\r"] {
        let code = format!(
            "fnc add_one(int x) -> int // comment{sep}  return x + 1;{sep}end{sep}pub fnc main(): int {{ return add_one(41); }}"
        );
        assert!(compile(&code).is_ok(), "comment swallowed the callable on {sep:?}");
    }
}

#[test]
fn separator_is_required_for_same_line_body_and_colon_returns_remain_braced() {
    for separator in ["", " /* only comment */ ", " // no newline "] {
        let source = format!(
            "fnc f() -> int {separator}return 42; end\npub fnc main(): int {{ return f(); }}"
        );
        let err = compile(&source).expect_err(&source);
        assert_eq!(err.code, "E_SYNTAX", "{source}");
    }
    for source in [
        "fnc f() : int :: return 42; end\npub fnc main(): int { return f(); }",
        "fnc f() -> int\n  return 42;",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    assert!(compile("fnc f() -> int :: return 42; end\npub fnc main(): int { return f(); }").is_ok());
}

#[test]
fn nested_end_is_not_the_end_of_a_free_callable() {
    let source = "fnc pick(int n) -> int\n  match n over\n    on 42 -> { return 42; }\n    on _ -> { return 0; }\n  end\nend\npub fnc main(): int { return pick(42); }";
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.funcs.len(), 2);
    assert!(compile(source).is_ok());
}
