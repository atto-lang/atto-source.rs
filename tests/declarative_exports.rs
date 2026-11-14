use oreslang_source::frontend::{parse, ExportScope};

// Native scalar scope: class/actor exports must fail rather than be treated
// as function exports or accidentally exposed through object linkage.
const DECL: &str = "fnc helper(): int { return 41; }\nfnc main(): int { return helper()+1; }\n";

#[test]
fn forward_and_backward_exports_have_the_same_symbol_semantics() {
    let top = parse(&format!("export pub fnc helper;\nexport pub main;\n{DECL}")).unwrap();
    let bottom = parse(&format!("{DECL}export pub fnc helper;\nexport pub main;\n")).unwrap();
    assert_eq!(top.functions, bottom.functions);
    assert_eq!(top.exports.iter().map(|e| (&e.name, e.scope, &e.kind)).collect::<Vec<_>>(),
               bottom.exports.iter().map(|e| (&e.name, e.scope, &e.kind)).collect::<Vec<_>>());
    assert!(top.functions.iter().any(|f| f.name == "helper" && f.exported));
}

#[test]
fn package_export_is_not_public_elf_linkage() {
    let p = parse(&format!("export pub[pkg] fnc helper;\n{DECL}")).unwrap();
    assert_eq!(p.exports[0].scope, ExportScope::Package);
    assert!(!p.functions.iter().find(|f| f.name == "helper").unwrap().exported);
}

#[test]
fn exports_fail_closed_on_unbound_wrong_kind_and_duplicate() {
    for (export, code) in [
        ("export pub Missing;", "export_unresolved"),
        ("export pub class helper;", "export_kind_mismatch"),
        ("export pub helper; export pub fnc helper;", "duplicate_export"),
        ("export pub[pkg] helper; pub fnc helper(): int { return 0; }", "export_visibility_conflict"),
    ] {
        let source = if export.contains("pub fnc helper():") {
            format!("{export}\nfnc main(): int {{ return helper(); }}")
        } else {
            format!("{export}\n{DECL}")
        };
        assert_eq!(parse(&source).unwrap_err().class, code, "{source}");
    }
}

#[test]
fn invalid_visibility_and_nested_export_are_rejected() {
    assert!(parse(&format!("export pub[world] helper;\n{DECL}")).is_err());
    assert!(parse("fnc main(): int { export pub main; return 0; }").is_err());
}
