use atto_compiler::lexer::lex;
const KEYWORDS: [&str;16] = ["trap!","throw!","raise!","panic!","copy!","copy!!",
    "take!","move!","borrow!","share!","pin!","drop!","pipe!","rpipe!","stop!","expand!"];
#[test]
fn shared_separator_matrix() {
    for k in KEYWORDS {
        for sep in [" ","\t","\n","\r\n","\x0c","\x0b"] {
            let source=format!("{k}{sep}value");
            assert!(lex(&source).is_ok(),"{source:?}");
        }
        for tail in ["value","(42)","/*comment*/ value","","\u{a0}value"] {
            let source=format!("{k}{tail}");
            assert_eq!(lex(&source).unwrap_err().code,"E_BANG_KEYWORD_WHITESPACE","{source:?}");
        }
    }
}
#[test]
fn preserve_inequality_and_opaque_text() {
    for source in ["copy!=value","copy!eq value","copy !value","x!value",
        "/* trap!value */ 42","// copy!!value\n42","\"trap!value\"","'copy!!value'"] {
        assert!(lex(source).is_ok(),"{source:?}");
    }
    assert_eq!(lex("copy!eqfoo").unwrap_err().code,"E_BANG_KEYWORD_WHITESPACE");
    assert_eq!(lex("copy!eqé").unwrap_err().code,"E_BANG_KEYWORD_WHITESPACE");
}
#[test]
fn compiler_negative_diagnostic_precedes_semantic_analysis() {
    for k in KEYWORDS {
        let source=format!("pub fnc main(): int {{ return {k}value; }}");
        assert_eq!(atto_compiler::compile(&source).unwrap_err().code,"E_BANG_KEYWORD_WHITESPACE","{source}");
    }
}
