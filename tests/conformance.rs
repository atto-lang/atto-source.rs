//! Inputs copied verbatim from Java/LLVM canonical semantic conformance
//! fixtures (October 19, 2026). See tests/CONFORMANCE.md for provenance.
use oreslang_source_rs::{compile, frontend, mir};

fn checked(source: &str) -> Result<(), String> {
    let parsed = frontend::parse(source).map_err(|e| e.to_string())?;
    mir::lower(parsed).map(|_| ()).map_err(|e| e.to_string())
}
#[test]
fn canonical_positive_native_compile() {
    for (name, source) in [
        ("add", include_str!("fixtures/add.ores")),
        ("precedence", include_str!("fixtures/precedence.ores")),
        ("forward", include_str!("fixtures/forward.ores")),
        ("underscores", include_str!("fixtures/underscores.ores")),
        ("unary", include_str!("fixtures/unary.ores")),
        ("owned-identifier", include_str!("fixtures/owned-identifier.ores")),
        ("nested-comments", include_str!("fixtures/nested-comments.ores")),
    ] {
        let result = compile(source);
        assert!(result.is_ok(), "{name}: {result:?}");
    }
}
#[test]
fn canonical_diagnostic_classes() {
    for (name, source, class) in [
        ("unknown", include_str!("fixtures/unknown.ores"), "unknown_name"),
        ("arity", include_str!("fixtures/arity.ores"), "call_arity"),
        ("duplicate", include_str!("fixtures/duplicate.ores"), "duplicate_declaration"),
        ("overflow", include_str!("fixtures/overflow.ores"), "integer_overflow"),
        ("comment", include_str!("fixtures/comment.ores"), "unterminated_comment"),
    ] {
        let error = checked(source).unwrap_err();
        assert!(error.starts_with(class), "{name}: expected {class}, got {error}");
    }
}
