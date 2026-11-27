use oreslang_source_rs::frontend;

#[test]
fn terminal_controls_are_rejected_before_comments_are_discarded() {
    for control in ['\u{001b}', '\u{007f}'] {
        for source in [
            format!("fnc main():int{{/*{control}*/return 42;}}"),
            format!("fnc main():int{{return 42;}}//{control}"),
        ] {
            let error = frontend::parse(&source).expect_err("ESC/DEL must fail admission");
            assert_eq!(error.class, "invalid_source_control");
            assert_eq!(error.offset, source.find(control).unwrap());
        }
    }
}

#[test]
fn ordinary_whitespace_is_not_a_terminal_control() {
    frontend::parse("fnc main():int{\r\n\treturn 42;\r\n}")
        .expect("CR/LF/tab are legitimate source whitespace");
}

#[test]
fn first_invalid_control_byte_wins() {
    for (controls, expected_class) in [
        ("\u{001b}\0", "invalid_source_control"),
        ("\0\u{007f}", "source_admission"),
        ("\u{007f}\u{001b}", "invalid_source_control"),
    ] {
        let source = format!("fnc main():int{{/*{controls}*/return 42;}}");
        let error = frontend::parse(&source).expect_err("controls must fail admission");
        let first = source.bytes().position(|byte| matches!(byte, 0 | 0x1b | 0x7f)).unwrap();
        assert_eq!(error.offset, first);
        assert_eq!(error.class, expected_class);
    }
}
