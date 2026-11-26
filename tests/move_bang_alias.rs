//! Real compiler admission and emitted-Wasm parity for the transfer alias.
use atto_compiler::{compile, lexer::lex};

#[test]
fn transfer_alias_emits_identical_wasm() {
    let source = "pub fnc main(): int { val x = 42; val y = %s x; return y; }";
    let take = compile(&source.replace("%s", "take!")).unwrap();
    let moved = compile(&source.replace("%s", "move!")).unwrap();
    assert_eq!(&take[..8], b"\0asm\x01\0\0\0");
    assert_eq!(take, moved);
}

#[test]
fn moved_scalar_cannot_be_read_again() {
    let source = include_str!("fixtures/fail/173-take-then-copy-scalar.atto");
    for op in ["take!", "move!"] {
        let err = compile(&source.replace("take!", op)).unwrap_err();
        assert_eq!(err.code, "E_MOVED", "{err}");
    }
}

#[test]
fn spacing_and_identifier_boundaries() {
    for op in ["take", "move"] {
        for suffix in ["!x", "!(x)", "!! x", "!", "!\u{a0}x"] {
            let error = lex(&format!("{op}{suffix}")).unwrap_err();
            assert_eq!(error.code, "E_BANG_KEYWORD_WHITESPACE", "{error}");
        }
        for suffix in ["! x", "!\nx", "!=x", "!eq x"] {
            assert!(lex(&format!("{op}{suffix}")).is_ok());
        }
        assert!(compile(&format!("pub fnc main(): int {{ val x = 42; return {op} ! x; }}")).is_err());
    }
    assert!(compile("pub fnc main(): int { val move = 42; return move; }").is_ok());
}
