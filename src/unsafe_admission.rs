//! G0 browser safety gate for proposed `unsafe { }` and `unsafe fnc`.
//! The lexer already masks strings/comments, so token matching is structural
//! at the lexical level. This is NOT an unsafe-block parser or guest lowering.
//! The gate must run before any alternate syntax/collection/AOT lowering.
use crate::diag::Diagnostic;
use crate::lexer::{Tok, Token};

pub fn reject_unlowered(tokens: &[Token]) -> Result<(), Diagnostic> {
    for token in tokens {
        if let Tok::Ident(word) = &token.tok {
            if word == "unsafe" {
                return Err(Diagnostic::new(
                    token.pos,
                    "E_UNSAFE_UNSUPPORTED",
                    "unsafe blocks and unsafe callables are reserved but not implemented in the browser/Wasm guest profile; no unsafe grant or native host operation is available",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile, lexer};

    #[test]
    fn deny_empty_and_nested_blocks_before_wasm_emission() {
        for source in [
            "pub fnc main(): int { unsafe { } return 42; }",
            "pub fnc main(): int { unsafe { unsafe { } } return 42; }",
            "pub unsafe fnc main(): int { return 42; }",
            "pub fnc main(): int { unsafe; return 42; }",
        ] {
            let err = compile(source).expect_err("no guest artifact for unsafe source");
            assert_eq!(err.code, "E_UNSAFE_UNSUPPORTED", "{source}");
        }
    }

    #[test]
    fn reject_diagnostic_uses_standalone_token_position() {
        let source = "pub fnc main(): int {\n    unsafe { }\n    return 42;\n}";
        let err = compile(source).expect_err("reserved unsafe must reject");
        assert_eq!(err.code, "E_UNSAFE_UNSUPPORTED");
        assert_eq!(err.pos.line, 2);
        assert_eq!(err.pos.col, 5);
    }

    #[test]
    fn ignore_literals_comments_and_identifier_substrings() {
        // The policy gate should never mistake a string/comment for an unsafe block.
        let source = "pub fnc main(): int { /* unsafe { */ val int unsafe_tools = 42; // unsafe\n return unsafe_tools; }";
        reject_unlowered(&lexer::lex(source).expect("lex safe source")).unwrap();
        let quoted = "pub fnc main(): int { val string s = \"unsafe { }\"; return 42; }";
        reject_unlowered(&lexer::lex(quoted).expect("lex quoted source")).unwrap();
        assert_eq!(compile(source).is_ok(), true, "ordinary safe program must still compile");
    }

    #[test]
    fn imported_or_prelexed_tokens_cannot_bypass_gate() {
        let tokens = lexer::lex("pub fnc main(): int { unsafe { } return 42; }").unwrap();
        let err = reject_unlowered(&tokens).unwrap_err();
        assert_eq!(err.code, "E_UNSAFE_UNSUPPORTED");
    }

    #[test]
    fn safe_lexer_keyword_near_misses_do_not_trigger_gate() {
        for source in ["unsafe_tools", "myunsafe", "_unsafe", "unsafe2", "\"unsafe\"", "/*unsafe*/"] {
            let tokens = lexer::lex(source).expect("safe near-miss should lex");
            assert!(reject_unlowered(&tokens).is_ok(), "{source}");
        }
    }
}
