//! Actual browser lexer and compiler raw source admission.
use atto_compiler::{compile, lexer};

fn check(source: &str, code: &str, line: u32, col: u32) {
    let error = lexer::lex(source).expect_err("raw source control admitted");
    assert_eq!(error.code, code);
    assert_eq!((error.pos.line, error.pos.col), (line, col));
    assert_eq!(compile(source).expect_err("compiled invalid raw source").code, code);
}

#[test]
fn terminal_controls_cannot_hide_inside_comments_or_strings() {
    for control in ['\u{001b}', '\u{007f}', '\0'] {
        let code = if control == '\0' { "E_NUL" } else { "E_SOURCE_CONTROL" };
        for source in [
            format!("pub fnc main():int{{/*{control}*/return 42;}}"),
            format!("pub fnc main():int{{return 42;}}//{control}"),
            format!("pub fnc main():int{{return 42;}}/* {control} */"),
            format!("/* café 😀 */{control}"),
            format!("pub fnc main():int{{val string s=\"{control}\";return 42;}}"),
        ] {
            let byte = source.find(control).unwrap();
            check(&source, code, 1, source[..byte].chars().count() as u32 + 1);
        }
    }
}

#[test]
fn first_forbidden_character_has_precise_browser_position() {
    for (source, code, line, col) in [
        ("\u{001b}\0", "E_SOURCE_CONTROL", 1, 1),
        ("\0\u{007f}", "E_NUL", 1, 1),
        ("x\r\n\u{007f}\0", "E_SOURCE_CONTROL", 2, 1),
        ("/* café 😀 */\u{001b}", "E_SOURCE_CONTROL", 1, 13),
        ("/* \u{202e} */\0", "E_SOURCE_BIDI", 1, 4),
    ] { check(source, code, line, col); }
}

#[test]
fn ordinary_whitespace_and_unicode_comments_remain_valid() {
    compile("/* café 😀 */ pub fnc main(): int {\r\n\treturn 42;\r\n}").unwrap();
}

#[test]
fn block_comment_depth_is_bounded() {
    let source = format!("pub fnc main(): int {{ {}{} return 42; }}", "/*".repeat(65), "*/".repeat(65));
    assert_eq!(lexer::lex(&source).unwrap_err().code, "E_SOURCE_LIMIT");
    assert_eq!(compile(&source).unwrap_err().code, "E_SOURCE_LIMIT");
}
