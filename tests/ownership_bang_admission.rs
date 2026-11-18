//! Exact frontend admission tests for ownership punctuation not yet lowered.
use oreslang_source_rs::frontend::parse;

#[test]
fn ownership_bangs_fail_closed_before_native_ir() {
    for expression in [
        "copy!42", "take!42", "borrow! item", "borrow!item",
        "borrow! mut item", "borrow!mut item", "mut borrow! item",
        "mut borrow!item", "share!item",
        "borrow!\nmut item", "borrow!/* checked loan */mut item",
        "mut borrow!\nitem",
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
fn nested_comments_obey_the_explicit_depth_budget() {
    let nested = "pub fnc main(): int { /* outer /* borrow! mut item */ tail */ return 42; }";
    assert!(parse(nested).is_ok());

    let too_deep = format!(
        "pub fnc main(): int {{ {}{} return 42; }}",
        "/*".repeat(65),
        "*/".repeat(65)
    );
    let error = parse(&too_deep).expect_err("nesting above 64 must be rejected");
    assert_eq!(error.class, "resource_limit");
    assert!(error.message.contains("block comment nesting exceeds 64"));
}
