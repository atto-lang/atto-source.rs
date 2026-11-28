//! Production compiler ownership punctuation admission and negative guest gates.
use atto_compiler::{compile, lexer::lex};

#[test]
fn unresolved_ownership_operands_fail_closed() {
    for expression in [
        "copy!item", "take!item", "borrow! item", "borrow!item",
        "borrow! mut item", "borrow!mut item", "mut borrow! item",
        "mut borrow!item", "share!item",
        "borrow!\nmut item", "borrow!/* checked loan */mut item",
        "mut borrow!\nitem",
    ] {
        let source = format!("pub fnc main(): int {{ return {expression}; }}");
        assert!(compile(&source).is_err(), "unbound ownership operand emitted guest: {source}");
    }
}

#[test]
fn scalar_copy_and_ordinary_comments_compile() {
    for source in [
        "pub fnc main(): int { return copy!42; }",
        "pub fnc main(): int { /* borrow! mut item */ return 42; }",
        "pub fnc main(): int { // borrow! mut item\n return 42; }",
        "pub fnc main(): int { /* outer /* borrow! mut item */ tail */ return 42; }",
    ] {
        let guest = compile(source).expect(source);
        assert!(guest.starts_with(&[0, 97, 115, 109, 1, 0, 0, 0]), "{source}");
    }
}

#[test]
fn spaced_punctuation_is_not_a_contextual_ownership_alias() {
    assert!(compile("pub fnc main(): int { val item = 42; return copy !item; }").is_err());
    assert!(compile("pub fnc main(): int { val item = 42; return borrow !item; }").is_err());
}

#[test]
fn nested_comments_obey_the_explicit_depth_budget() {
    let at_limit = format!("pub fnc main(): int {{ {}{} return 42; }}", "/*".repeat(64), "*/".repeat(64));
    assert!(lex(&at_limit).is_ok());
    let too_deep = format!("pub fnc main(): int {{ {}{} return 42; }}", "/*".repeat(65), "*/".repeat(65));
    let error = lex(&too_deep).expect_err("nesting above 64 must be rejected");
    assert_eq!(error.code, "E_SOURCE_LIMIT");
    assert!(error.message.contains("block comment nesting exceeds 64"));
    assert!(compile(&too_deep).is_err());
}
