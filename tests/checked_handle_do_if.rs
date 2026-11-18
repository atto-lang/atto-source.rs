//! Verified do-if loan preservation through the production Atto checker and
//! the independent bounded all-path checked-handle CFG adapter.
//!
//! This is a compiler/guest profile only. Cross-actor arena and suspension
//! contracts are still rejected until runtime+typed-MIR integration.

#[test]
fn do_if_scoped_readonly_branches_preserve_handle_and_restore_owned_class() {
    for source in [
        include_str!("fixtures/pass/checked-handle-do-if-branch-v1.atto"),
        include_str!("fixtures/pass/checked-handle-do-if-nested-v1.atto"),
        include_str!("fixtures/pass/checked-handle-do-if-branch-local-views-v1.atto"),
    ] {
        let artifact = atto_compiler::compile(source)
            .unwrap_or_else(|e| panic!("do if checked-handle proof rejected: {e}"));
        assert_eq!(&artifact[..8], b"\0asm\x01\0\0\0");
        assert_eq!(atto_compiler::compile(source).unwrap(), artifact,
            "do-if compiled guest bytes must be deterministic");
    }
}

#[test]
fn do_if_rejects_early_return_release_escape_and_transitive_throw() {
    for (source, code) in [
        (include_str!("fixtures/fail/checked-handle-do-if-early-return.atto"), "E_HANDLE"),
        (include_str!("fixtures/fail/checked-handle-do-if-unref-in-arm.atto"), "E_HANDLE"),
        (include_str!("fixtures/fail/checked-handle-do-if-escape-view.atto"), "E_BORROW"),
        (include_str!("fixtures/fail/checked-handle-do-if-transitive-throw.atto"), "E_HANDLE"),
    ] {
        let diagnostic = atto_compiler::compile(source).expect_err(
            "unverified do-if ownership branch must not emit guest bytes");
        assert_eq!(diagnostic.code, code, "{diagnostic}");
    }
}
