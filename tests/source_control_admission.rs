//! Raw terminal control rejection before comment preprocessing.
use atto_compiler::{compile, lexer::lex};

fn pos_at(source: &str, offset: usize) -> (u32,u32) {
    let prefix = &source[..offset];
    (1 + prefix.chars().filter(|&c| c == '\n').count() as u32,
     1 + prefix.rsplit('\n').next().unwrap().chars().count() as u32)
}

#[test]
fn terminal_controls_are_rejected_in_comments_and_other_source_regions() {
    for control in ['\u{001b}', '\u{007f}'] {
        for source in [
            format!("fnc main():int{{/*{control}*/return 42;}}"),
            format!("fnc main():int{{return 42;}}//{control}"),
            format!("fnc main():int{{return 42;}}/* {control} */"),
            format!("/* café 😀 */{control}"),
        ] {
            let expected = source.find(control).expect("injected control");
            let error = lex(&source).expect_err("terminal control must be rejected");
            assert_eq!(error.code, "E_SOURCE_CONTROL");
            assert_eq!((error.pos.line, error.pos.col), pos_at(&source, expected));
            assert!(compile(&source).is_err());
            assert_eq!(error.message, "source contains terminal control");
        }
    }
}

#[test]
fn ordinary_whitespace_and_unicode_comments_remain_valid() {
    lex("/* café 😀 */ pub fnc main(): int {\r\n\treturn 42;\r\n}")
        .expect("valid UTF-8 comment and whitespace");
}

#[test]
fn first_forbidden_byte_wins_and_offset_is_utf8_byte_based() {
    for (source, expected_message, expected_offset) in [
        ("\u{001b}\0", "source contains terminal control", 0),
        ("\0\u{007f}", "source contains NUL byte", 0),
        ("x\r\n\u{007f}\0", "source contains terminal control", 3),
        ("/* café 😀 */\u{001b}", "source contains terminal control", 16),
    ] {
        let error = lex(source).expect_err("invalid raw byte admitted");
        let code = if expected_message.contains("NUL") { "E_NUL" } else { "E_SOURCE_CONTROL" };
        assert_eq!(error.code, code);
        assert!(error.message.contains(if code == "E_NUL" { "NUL" } else { "terminal control" }));
        assert_eq!((error.pos.line, error.pos.col), pos_at(source, expected_offset));
    }
}

#[test]
fn block_comment_depth_is_bounded_before_lexer_processing() {
    let input = format!(
        "pub fnc main(): int {{ {}{} return 42; }}",
        "/*".repeat(65),
        "*/".repeat(65)
    );
    let error = lex(&input).expect_err("too much comment nesting must be denied");
    assert_eq!(error.code, "E_SOURCE_LIMIT");
}
