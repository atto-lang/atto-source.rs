//! Raw source admission in the browser compiler, before lexical preprocessing.
use atto_compiler::{compile, lexer::lex, diag::Pos};

#[test]
fn terminal_controls_are_rejected_before_comments_and_literals() {
    for control in ['\u{001b}', '\u{007f}', '\0'] {
        let code = if control == '\0' { "E_NUL" } else { "E_SOURCE_CONTROL" };
        for source in [
            format!("pub fnc main(): int {{ /*{control}*/return 42; }}"),
            format!("pub fnc main(): int {{ return 42; }}//{control}"),
            format!("pub fnc main(): int {{ val s = \"{control}\"; return 42; }}"),
            format!("/* café 😀 */{control}"),
        ] {
            let error = lex(&source).expect_err("raw control must be rejected");
            assert_eq!(error.code, code);
            assert_eq!(error.pos, Pos { line: 1, col: source.chars().position(|c| c == control).unwrap() as u32 + 1 });
            assert_eq!(compile(&source).unwrap_err().code, code);
        }
    }
}
#[test]
fn valid_unicode_comments_and_whitespace_remain_admitted() {
    let source = "/* café 😀 */ pub fnc main(): int {\r\n\treturn 42;\r\n}";
    assert!(compile(source).is_ok());
}
#[test]
fn first_raw_control_wins_and_columns_are_character_based() {
    for (source, code, line, col) in [
        ("\u{001b}\0", "E_SOURCE_CONTROL", 1, 1),
        ("\0\u{007f}", "E_NUL", 1, 1),
        ("x\r\n\u{007f}\0", "E_SOURCE_CONTROL", 2, 1),
        ("/* café 😀 */\u{001b}", "E_SOURCE_CONTROL", 1, 13),
    ] {
        let error = lex(source).unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.pos, Pos { line, col });
    }
}
#[test]
fn nested_comment_budget_is_enforced() {
    let source = format!("pub fnc main(): int {{ {}{} return 42; }}",
        "/*".repeat(65), "*/".repeat(65));
    assert_eq!(lex(&source).unwrap_err().code, "E_SOURCE_LIMIT");
}
