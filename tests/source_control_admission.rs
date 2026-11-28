//! Raw terminal control rejection before comment preprocessing.
//! The browser compiler is atto_compiler, not the legacy host frontend.
use atto_compiler::{compile, lexer::lex, Pos};

fn source_pos(source: &str, offset: usize) -> Pos {
    let mut line = 1;
    let mut col = 1;
    for c in source[..offset].chars() {
        if c == '\n' { line += 1; col = 1; }
        else { col += 1; }
    }
    Pos { line, col }
}

#[test]
fn terminal_controls_are_rejected_in_comments_and_other_source_regions() {
    for control in ['\u{001b}', '\u{007f}', '\0'] {
        let code = if control == '\0' { "E_NUL" } else { "E_SOURCE_CONTROL" };
        for source in [
            format!("pub fnc main():int{{/*{control}*/return 42;}}"),
            format!("pub fnc main():int{{return 42;}}//{control}"),
            format!("pub fnc main():int{{return 42;}}/* {control} */"),
            format!("/* café 😀 */{control}"),
        ] {
            let expected = source_pos(&source, source.find(control).unwrap());
            let error = lex(&source).expect_err("terminal control must be rejected");
            assert_eq!(error.code, code);
            assert_eq!(error.pos, expected);
            assert_eq!(compile(&source).expect_err("no compiled artifact").code, code);
        }
    }
}

#[test]
fn ordinary_whitespace_and_unicode_comments_remain_valid() {
    compile("/* café 😀 */ pub fnc main(): int {\r\n\treturn 42;\r\n}")
        .expect("valid UTF-8 comment and whitespace");
}

#[test]
fn first_forbidden_character_wins_with_unicode_source_positions() {
    for (source, code, expected) in [
        ("\u{001b}\0", "E_SOURCE_CONTROL", Pos { line: 1, col: 1 }),
        ("\0\u{007f}", "E_NUL", Pos { line: 1, col: 1 }),
        ("x\r\n\u{007f}\0", "E_SOURCE_CONTROL", Pos { line: 2, col: 1 }),
        ("/* café 😀 */\u{001b}", "E_SOURCE_CONTROL", Pos { line: 1, col: 13 }),
        ("\u{202e}\0", "E_SOURCE_BIDI", Pos { line: 1, col: 1 }),
    ] {
        let error = lex(source).expect_err("invalid raw source admitted");
        assert_eq!(error.code, code);
        assert_eq!(error.pos, expected);
    }
}

#[test]
fn block_comment_depth_is_bounded_before_parser_processing() {
    let input = format!(
        "pub fnc main(): int {{ {}{} return 42; }}",
        "/*".repeat(65),
        "*/".repeat(65)
    );
    let error = compile(&input).expect_err("too much comment nesting must be denied");
    assert_eq!(error.code, "E_SOURCE_LIMIT");
}
