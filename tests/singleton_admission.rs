//! Real canonical Rust browser frontend admission, not singleton guest implementation.
use atto_compiler::{compile, compile_verified_strict, compile_with_path, lexer::lex, parser::Parser};
use std::path::Path;

fn reject_at(source: &str) -> atto_compiler::diag::Diagnostic {
    match Parser::new(lex(source).expect("lex source")).parse_program() {
        Ok(_) => panic!("singleton declaration admitted: {source}"),
        Err(error) => error,
    }
}

#[test]
fn singleton_declarations_have_stable_fail_closed_diagnostic() {
    for declaration in [
        "define singleton S as end",
        "define pub singleton S as end",
        "pub define singleton S as end",
        "define /* inert */ singleton S as end",
        "define pub /* inert */ singleton S as end",
        "define module M as define singleton S as end end",
        "pub fnc main(): int { return 7; } define singleton S as end",
        "define singleton S as state { let n: int = 0 } end",
        "// comment\ndefine singleton S as end",
    ] {
        let err = reject_at(declaration);
        assert_eq!(err.code, "E_SINGLETON_UNSUPPORTED", "{declaration}: {err}");
        assert!(err.message.contains("ORES-SINGLETON-UNSUPPORTED"));
        let prefix = &declaration[..declaration.find("singleton").unwrap()];
        assert_eq!(err.pos.line, prefix.lines().count().max(1) as u32);
        assert_eq!(err.pos.col, prefix.rsplit('\n').next().unwrap().chars().count() as u32 + 1);
    }
}

#[test]
fn singleton_is_contextual_and_comment_text_is_inert() {
    for source in [
        "fnc singleton(): int { return 7; }",
        "fnc main(int singleton): int { return singleton; }",
        "pub fnc main(): int { return singleton(); } fnc singleton(): int { return 7; }",
        "// define singleton Fake as end\nfnc main(): int { return 7; }",
        "/* define pub singleton Fake as end */ fnc main(): int { return 7; }",
        "fnc main(): int { return 7; } // define singleton Fake",
    ] {
        Parser::new(lex(source).unwrap()).parse_program()
            .unwrap_or_else(|e| panic!("ordinary source denied {source}: {e}"));
    }
    let bytes = compile("pub fnc main(): int { return singleton(); } fnc singleton(): int { return 7; }").unwrap();
    assert!(bytes.starts_with(b"\0asm"));
}

#[test]
fn unsupported_singleton_cannot_enter_canonical_code_generation() {
    let src = "pub fnc main(): int { return 7; }\ndefine singleton S as end";
    for result in [
        compile(src),
        compile_verified_strict(src),
        compile_with_path(src, Path::new("main.atto")),
    ] {
        let err = result.unwrap_err();
        assert_eq!(err.code, "E_SINGLETON_UNSUPPORTED");
        assert!(err.message.contains("ORES-SINGLETON-UNSUPPORTED"));
        assert_eq!((err.pos.line, err.pos.col), (2, 8));
    }
}
