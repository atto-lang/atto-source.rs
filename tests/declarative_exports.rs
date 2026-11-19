use oreslang_source_rs::frontend::{parse, ExportScope};

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

#[test]
fn mir_preserves_package_manifest_without_public_linkage() {
    let ast = parse(&format!("export pub[pkg] fnc helper;\n{DECL}")).unwrap();
    let typed = oreslang_source_rs::mir::lower(ast).expect("validated MIR");
    assert_eq!(typed.exports.len(), 1, "pkg visibility must survive lower");
    assert_eq!(typed.exports[0].name, "helper");
    assert_eq!(typed.exports[0].scope, ExportScope::Package);
    assert!(!typed.functions.iter().find(|f| f.symbol == "helper").unwrap().exported);
}

#[test]
fn package_export_remains_a_nonpublic_tree_shaker_root() {
    let source = "export pub[pkg] fnc package_api;\n\
pub fnc main(): int { return 42; }\n\
fnc package_api(): int { return 7; }\n\
fnc dead_private(): int { return 9; }";
    for profile in [
        oreslang_source_rs::profile::BuildProfile::Release,
        oreslang_source_rs::profile::BuildProfile::StrictAot,
    ] {
        let program = parse(source).unwrap();
        let module = oreslang_source_rs::mir::lower(program).unwrap();
        let shaken = oreslang_source_rs::profile::apply(module, profile).unwrap();
        let symbols: Vec<_> = shaken
            .functions
            .iter()
            .map(|function| function.symbol.as_str())
            .collect();
        assert!(symbols.contains(&"main"), "{profile:?}: main removed");
        assert!(
            symbols.contains(&"package_api"),
            "{profile:?}: package export was incorrectly pruned"
        );
        assert!(
            !symbols.contains(&"dead_private"),
            "{profile:?}: ordinary private function was retained"
        );
        assert!(
            !shaken
                .functions
                .iter()
                .find(|function| function.symbol == "package_api")
                .unwrap()
                .exported,
            "package API must not become public native linkage"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn strict_aot_native_object_keeps_package_export_local() {
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "oresc-pkg-export-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("package.atto");
    let object = dir.join("package.o");
    fs::write(
        &source,
        "export pub[pkg] fnc package_api;\n\
pub fnc main(): int { return 42; }\n\
fnc package_api(): int { return 7; }\n\
fnc dead_private(): int { return 9; }\n",
    )
    .unwrap();

    let run = Command::new(env!("CARGO_BIN_EXE_oresc"))
        .args(["--strict-aot"])
        .arg(&source)
        .arg(&object)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let output = Command::new("nm")
        .arg("--defined-only")
        .arg(&object)
        .output()
        .unwrap();
    assert!(output.status.success());
    let symbols = String::from_utf8(output.stdout).unwrap();
    let package_line = symbols
        .lines()
        .find(|line| line.split_whitespace().last() == Some("package_api"))
        .expect("package export was stripped from native object");
    let parts: Vec<_> = package_line.split_whitespace().collect();
    assert_eq!(parts[parts.len() - 2], "t", "package API leaked as public: {symbols}");
    assert!(
        !symbols.contains("dead_private"),
        "ordinary private dead function was not pruned: {symbols}"
    );
    fs::remove_dir_all(dir).unwrap();
}
