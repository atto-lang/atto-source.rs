//! G0 guest admission only; no executable native unsafe or trusted effects.
use atto_compiler::{compile, compile_verified_strict, compile_with_imports, lexer, parser, Pos};
use std::path::Path;

#[test]
fn unsafe_guest_rejected_in_ordinary_and_strict_lanes() {
    for source in [
        "pub fnc main(): int { unsafe { } return 42; }",
        "pub unsafe fnc main(): int { return 42; }",
        "pub fnc main(): int { return 42; } // inert\runsafe",
        "import module innocent from \"atto::std::unsafe::ffi\"; pub fnc main(): int { return 42; }",
        "import module innocent from \"std/other/../unsafe/ffi\"; pub fnc main(): int { return 42; }",
    ] {
        assert_eq!(compile(source).unwrap_err().code, "E_UNSAFE_DOMAIN", "{source}");
        assert_eq!(compile_verified_strict(source).unwrap_err().code, "E_UNSAFE_DOMAIN", "{source}");
    }
    assert!(compile("pub fnc main(): int { /* unsafe */ val int unsafe_tools = 42; return unsafe_tools; }").is_ok());
    assert!(lexer::lex("pub fnc main(): int { val string s = \"std/unsafe/ffi\"; return 42; }").is_ok());
}

#[test]
fn forged_token_and_imported_source_cannot_bypass_gate() {
    let tokens = vec![
        lexer::Token { tok: lexer::Tok::Ident("unsafe".into()), pos: Pos {line: 1, col: 1} },
        lexer::Token { tok: lexer::Tok::Eof, pos: Pos {line: 1, col: 7} },
    ];
    assert_eq!(parser::Parser::new(tokens).parse_program().unwrap_err().code, "E_UNSAFE_DOMAIN");
    let result = compile_with_imports(
        "import value from \"./dep.atto\"; pub fnc main(): int { return value(); }",
        Path::new("main.atto"),
        |_| Ok("pub fnc value(): int { unsafe { } return 42; }".to_string()),
    );
    assert_eq!(result.unwrap_err().code, "E_UNSAFE_DOMAIN");
}
