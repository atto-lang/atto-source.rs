//! Compiler-backed admission: Unicode controls must not disappear inside comments.
//! This tests the actual Rust browser lexer and Wasm compiler, not Wasm module emission.
use atto_compiler::{compile, lexer::lex};
fn pos_at(source: &str, offset: usize) -> (u32,u32) {
    let prefix = &source[..offset];
    (1 + prefix.chars().filter(|&c| c == '\n').count() as u32,
     1 + prefix.rsplit('\n').next().unwrap().chars().count() as u32)
}
fn expected_code(c: char) -> &'static str {
    match c { '\0' => "E_NUL", '\u{001b}' | '\u{007f}' | '\u{0085}' | '\u{2028}' | '\u{2029}' => "E_SOURCE_CONTROL", _ => "E_SOURCE_BIDI" }
}

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
            let err = lex(&source).expect_err("source control was admitted");
            assert_eq!(err.code, expected_code(character), "{source:?}");
            assert_eq!((err.pos.line, err.pos.col), pos_at(&source, position), "{source:?}");
            assert!(compile(&source).is_err(), "Wasm module compiled: {source:?}");
        }
    }
}

#[test]
fn ordinary_unicode_comments_still_compile() {
    let source = format!("/* café ☕ */ {GOOD}");
    assert!(!lex(&source).unwrap().is_empty());
    assert!(!compile(&source).unwrap().is_empty());
}

/// Mixed control classes must report the first offending character, not the
/// later raw byte that happened to be checked by a separate admission pass.
#[test]
fn earliest_control_precedes_later_different_control_class() {
    for (source, expected_char, _expected_class, _expected_message) in [
        ("\u{202e}\0", '\u{202e}', "source_unicode_control", "U+202E"),
        ("\0\u{202e}", '\0', "source_admission", "NUL byte"),
        ("/* café */\u{2066}\u{001b}", '\u{2066}', "source_unicode_control", "U+2066"),
        ("/* café */\u{001b}\u{2066}", '\u{001b}', "source_admission", "terminal control"),
        ("// label\u{061c}\n\u{007f}", '\u{061c}', "source_unicode_control", "U+061C"),
        ("// label\u{007f}\n\u{061c}", '\u{007f}', "source_admission", "terminal control"),
    ] {
        let err = lex(source).expect_err("control unexpectedly admitted");
        assert_eq!(err.code, expected_code(expected_char), "{source:?}: {err}");
        assert_eq!((err.pos.line, err.pos.col), pos_at(source, source.find(expected_char).unwrap()), "{source:?}");
        assert!(compile(source).is_err(), "Wasm module published: {source:?}");
    }
}
