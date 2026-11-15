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
