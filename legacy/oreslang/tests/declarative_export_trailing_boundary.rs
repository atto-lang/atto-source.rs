use oreslang_source_rs::frontend::{parse, ExportScope};

const FUNCTIONS: &str =
    "fnc helper(): int { return 41; }\nfnc main(): int { return helper() + 1; }\n";

#[test]
fn trailing_package_export_does_not_escape_as_native_public_symbol() {
    let parsed = parse(&format!("{FUNCTIONS}export pub[pkg] fnc helper;")).unwrap();
    assert_eq!(parsed.exports.len(), 1);
    assert_eq!(parsed.exports[0].scope, ExportScope::Package);
    assert!(!parsed.functions.iter().find(|f| f.name == "helper").unwrap().exported);

    let lowered = oreslang_source_rs::mir::lower(parsed).unwrap();
    assert_eq!(lowered.exports[0].scope, ExportScope::Package);
    assert!(!lowered.functions.iter().find(|f| f.symbol == "helper").unwrap().exported);
}

#[test]
fn trailing_public_export_cannot_override_package_only_visibility() {
    for directives in [
        "export pub[pkg] fnc helper;\nexport pub fnc helper;",
        "export pub fnc helper;\nexport pub[pkg] fnc helper;",
    ] {
        assert_eq!(
            parse(&format!("{FUNCTIONS}{directives}")).unwrap_err().class,
            "duplicate_export"
        );
    }
}
