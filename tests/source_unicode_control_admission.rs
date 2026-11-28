//! Compiler-backed admission: Unicode controls must not disappear inside comments.
//! This tests the browser lexer and actual Wasm compiler; original native suite preserved.
use atto_compiler::{compile, lexer};

const GOOD: &str = "pub fnc main(): int { return 42; }";
const FORBIDDEN: [char; 15] = [
    '\u{0085}', '\u{061c}', '\u{200e}', '\u{200f}',
    '\u{2028}', '\u{2029}', '\u{202a}', '\u{202b}',
    '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}',
    '\u{2067}', '\u{2068}', '\u{2069}',
];

#[test]
fn forbidden_controls_are_rejected_before_both_comment_forms() {
    for character in FORBIDDEN {
        let cases = [
            format!("{GOOD} // hidden{character}return 0;"),
            format!("/* hidden{character} */ {GOOD}"),
            format!("// hidden{character}\n{GOOD}"),
        ];
        for source in cases {
            let position = source.find(character).expect("control exists");
            let err = lexer::lex(&source).expect_err("source control was admitted");
            let code = if matches!(character, '\u{0085}' | '\u{2028}' | '\u{2029}') { "E_SOURCE_UNICODE_CONTROL" } else { "E_SOURCE_BIDI" };
            assert_eq!(err.code, code, "{source:?}");
            assert_eq!(err.pos.col, source[..position].chars().count() as u32 + 1, "{source:?}");
            assert!(compile(&source).is_err(), "Wasm compiled: {source:?}");
        }
    }
}

#[test]
fn ordinary_unicode_comments_still_compile() {
    let source = format!("/* café ☕ */ {GOOD}");
    assert!(compile(&source).unwrap().starts_with(b"\0asm"));
}

/// Mixed control classes must report the first offending character, not the
/// later raw byte that happened to be checked by a separate admission pass.
#[test]
fn earliest_control_precedes_later_different_control_class() {
    for (source, expected_char, expected_class, expected_message) in [
        ("\u{202e}\0", '\u{202e}', "E_SOURCE_BIDI", "bidirectional Unicode control"),
        ("\0\u{202e}", '\0', "E_NUL", "NUL characters"),
        ("/* café */\u{2066}\u{001b}", '\u{2066}', "E_SOURCE_BIDI", "bidirectional Unicode control"),
        ("/* café */\u{001b}\u{2066}", '\u{001b}', "E_SOURCE_CONTROL", "terminal control"),
        ("// label\u{061c}\n\u{007f}", '\u{061c}', "E_SOURCE_BIDI", "bidirectional Unicode control"),
        ("// label\u{007f}\n\u{061c}", '\u{007f}', "E_SOURCE_CONTROL", "terminal control"),
    ] {
        let err = lexer::lex(source).expect_err("control unexpectedly admitted");
        assert_eq!(err.code, expected_class, "{source:?}: {err}");
        assert_eq!(err.pos.col, source[..source.find(expected_char).unwrap()].chars().count() as u32 + 1, "{source:?}");
        assert!(err.message.contains(expected_message), "{source:?}: {err}");
        assert!(compile(source).is_err(), "Wasm published: {source:?}");
    }
}
