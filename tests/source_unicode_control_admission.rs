//! Compiler-backed admission: Unicode controls must not disappear inside comments.
//! This tests the Rust scalar frontend/object emitter only, not other backends.
use oreslang_source_rs::{compile, frontend};

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
            format!("// hidden{character}\\n{GOOD}"),
        ];
        for source in cases {
            let position = source.find(character).expect("control exists");
            let err = frontend::parse(&source).expect_err("source control was admitted");
            assert_eq!(err.class, "source_unicode_control", "{source:?}");
            assert_eq!(err.offset, position, "{source:?}");
            assert!(err.message.contains("forbidden source Unicode control"));
            assert!(compile(&source).is_err(), "native object compiled: {source:?}");
        }
    }
}

#[test]
fn ordinary_unicode_comments_still_compile() {
    let source = format!("/* café ☕ */ {GOOD}");
    assert_eq!(frontend::parse(&source).unwrap().functions.len(), 1);
    assert!(!compile(&source).unwrap().is_empty());
}
