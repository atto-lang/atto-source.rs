//! Namespace identity gates for independent core/option and core/result sources.
//! This is lexical/source-identity admission, not generic library guest proof.
use atto_compiler::{lexer::{lex, Tok}, namespace};
use std::path::Path;

fn check_header(source: &str, logical_path: &str) {
    let tokens = lex(source).expect("real .atto library must lex");
    let normalized = namespace::normalize(&tokens, Some(Path::new(logical_path)))
        .expect("source namespace must be well-formed");
    let header: Vec<&Tok> = normalized.iter().take(4).map(|token| &token.tok).collect();
    assert!(matches!(header[0], Tok::Ident(word) if word == "use"));
    assert!(matches!(header[1], Tok::Ident(word) if word == "namespace"));
    assert!(matches!(header[2], Tok::Ident(word) if word == "core"));
    assert!(matches!(header[3], Tok::Semi));
    assert!(normalized.iter().any(|token| matches!(&token.tok, Tok::Ident(word) if word == "module")));
    assert!(!normalized.iter().any(|token| matches!(&token.tok, Tok::Ident(word) if word == "import")),
        "declaring a namespace must not silently pull in unrelated modules");
}

#[test]
fn backend_owned_option_and_result_use_the_same_core_namespace() {
    check_header(include_str!("../core/option.atto"), "core/option.atto");
    check_header(include_str!("../core/result.atto"), "core/result.atto");
}

#[test]
fn a_second_or_late_namespace_is_rejected_without_guest_emission() {
    for (source, code) in [
        ("use namespace core;\nuse namespace forged;\n", "E_DUPLICATE"),
        ("import module external from \"./foreign.atto\";\nuse namespace core;\n", "E_SYNTAX"),
    ] {
        let tokens = lex(source).expect("source is lexically valid");
        let diagnostic = namespace::normalize(&tokens, Some(Path::new("core/option.atto")))
            .expect_err("invalid namespace should not normalize");
        assert_eq!(diagnostic.code, code, "{diagnostic}");
    }
}
