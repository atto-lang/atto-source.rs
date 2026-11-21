//! Candidate for the 14th exact byte-verified website preview.
//! We correct private access *at the call site contract* in the website PR;
//! never weaken the ordinary compiler's private method checker.
use atto_compiler::compile;

const SITE: &str = include_str!("playground_previews/callable-body-styles.atto");

#[test]
fn exact_corrected_website_callable_styles_compile_into_deterministic_guest() {
    let bytes = compile(SITE).unwrap_or_else(|e| panic!("corrected exact site source must compile: {e}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(compile(SITE).unwrap(), bytes);
}

#[test]
fn without_explicit_public_inline_method_compilation_fails_closed() {
    let private = SITE.replace("  pub inline_end() -> int", "  inline_end() -> int");
    assert_ne!(private, SITE, "fixture must contain public inline method");
    let error = compile(&private).expect_err("main must not call private inline_end");
    assert_eq!(error.code, "E_PRIVATE", "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0);
}

#[test]
fn without_explicit_public_receiver_method_compilation_fails_closed() {
    let private = SITE.replace(
        "pub mtd (self: Calculator) receiver_end() -> int",
        "mtd (self: Calculator) receiver_end() -> int"
    );
    assert_ne!(private, SITE, "fixture must contain public out-of-line receiver");
    let error = compile(&private).expect_err("main must not call private receiver_end");
    assert_eq!(error.code, "E_PRIVATE", "{error}");
    assert!(error.pos.line > 0 && error.pos.col > 0);
}

#[test]
fn compiler_recovers_after_private_receiver_rejection() {
    let rejected = SITE.replace(
        "pub mtd (self: Calculator) receiver_end() -> int",
        "mtd (self: Calculator) receiver_end() -> int"
    );
    assert!(compile(&rejected).is_err());
    assert!(compile(SITE).is_ok(), "no stale class-symbol or receiver state across compilations");
}
