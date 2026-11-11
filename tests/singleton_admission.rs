use oreslang_source_rs::frontend;

#[test]
fn singleton_declarations_reject_before_any_native_scalar_lowering() {
    for source in [
        "define singleton Settings as const int x = 1; end",
        "define pub singleton Settings as const int x = 1; end",
        "pub define singleton Settings as const int x = 1; end",
        "define singleton Settings as end",
    ] {
        let error = frontend::parse(source).expect_err("singleton declaration must fail closed");
        assert_eq!(error.class, "singleton_unsupported", "{source}: {error}");
        assert_eq!(error.offset, source.find("singleton").unwrap());
    }
}

#[test]
fn comments_and_contextual_identifiers_do_not_forge_a_singleton_decl() {
    for source in [
        "// define singleton Cache as\nstatic fnc main() -> int { return 42; }",
        "/* define singleton Cache as */ static fnc main() -> int { return 42; }",
        "static fnc singleton() -> int { return 42; }",
        "static fnc main(int singleton_count) -> int { return singleton_count; }",
    ] {
        frontend::parse(source).unwrap_or_else(|e| {
            panic!("inert singleton spelling must remain legal: {source}: {e}");
        });
    }
}
