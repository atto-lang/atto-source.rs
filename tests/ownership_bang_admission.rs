//! Native Rust parser admission, not ownership MIR or executed actor guest parity.
use oreslang_source_rs::frontend::parse;

#[test]
fn ownership_bangs_fail_closed_before_native_ir() {
    for expression in [
        "copy!42", "take!42", "borrow! item", "borrow!item",
        "borrow! mut item", "borrow!mut item", "mut borrow! item", "mut borrow!item", "share!item",
    ] {
        let source = format!("pub fnc main(): int {{ return {expression}; }}");
        let error = parse(&source).expect_err(&source);
        assert_eq!(error.class, "ownership_bang_unsupported", "{source}: {error}");
        assert!(error.message.contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED"));
    }
}

#[test]
fn comments_and_ordinary_scalar_compilation_remain_unaffected() {
    for source in [
        "pub fnc main(): int { /* borrow! mut item */ return 42; }",
        "pub fnc main(): int { // borrow! mut item\n return 42; }",
    ] {
        assert!(parse(source).is_ok(), "{source}");
    }
}

#[test]
fn spaced_punctuation_is_not_a_contextual_ownership_alias() {
    let error = parse("pub fnc main(): int { return borrow !item; }").unwrap_err();
    assert_ne!(error.class, "ownership_bang_unsupported");
}

#[test]
fn nested_comments_cannot_escape_ownership_classifier_or_depth_budget() {
    let nested = "pub fnc main(): int { /* outer /* borrow! mut item */ tail */ return 42; }";
    assert!(parse(nested).is_ok(), "nested comments should not create ownership tokens");

    let unclosed = parse("pub fnc main(): int { /* outer /* copy!42 */ return 42; }")
        .expect_err("unclosed nested comment must be rejected");
    assert_eq!(unclosed.class, "unterminated_comment");

    let deeply_nested = format!(
        "pub fnc main(): int {{ {}{} return 42; }}",
        "/*".repeat(65),
        "*/".repeat(65)
    );
    let error = parse(&deeply_nested).expect_err("nesting above 64 must be rejected");
    assert_eq!(error.class, "resource_limit", "{error}");
    assert!(error.message.contains("block comment nesting exceeds 64"));
}
