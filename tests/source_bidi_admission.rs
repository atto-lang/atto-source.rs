use oreslang_source_rs::{compile, frontend};

const BIDI: [char; 12] = [
    '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}',
    '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
    '\u{2068}', '\u{2069}',
];
const SAFE: &str = "pub fnc main(): int { return 42; }";

#[test]
fn source_controls_are_refused_before_comments_are_removed() {
    for control in BIDI {
        for source in [
            format!("{SAFE} // {control}"),
            format!("/* {control} */ {SAFE}"),
        ] {
            let err = frontend::parse(&source).expect_err("bidi control was admitted");
            assert_eq!(err.class, "unsafe_unicode");
            assert!(err.to_string().contains("bidirectional Unicode"));
            assert!(compile(&source).is_err(), "object compiler bypassed source admission");
        }
    }
}

#[test]
fn ordinary_non_ascii_block_comment_remains_admitted() {
    assert!(frontend::parse(&format!("/* café */ {SAFE}")).is_ok());
}
