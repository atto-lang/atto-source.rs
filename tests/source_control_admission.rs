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
