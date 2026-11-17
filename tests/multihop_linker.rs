//! Multi-hop namespace module graph tests (real guest compiler).
use atto_compiler::linker::{compile_linked, LinkUnit};

const MAIN: &str = r#"
use namespace web;
import module Orders from "+lib/orders.atto";
pub fnc main(): int { return Orders.total(21); }
"#;
const ORDERS: &str = r#"
use namespace sales;
import module Math from "./math.atto";
export pub module Orders;
define module Orders as
    pub fnc total(int x): int { return Math.double(x); }
end
"#;
const MATH: &str = r#"
use namespace maths;
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
    fnc secret(): int { return 91; }
end
"#;
const PKG_MATH: &str = r#"
use namespace maths;
export pub[pkg] module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
end
"#;
fn fixture<'a>(main: &'a str, orders: &'a str, math: &'a str,
               main_pkg: &'a str, orders_pkg: &'a str, math_pkg: &'a str) -> [LinkUnit<'a>; 3] {
    [
      LinkUnit { path:"src/main.atto", package:main_pkg, source:main },
      LinkUnit { path:"lib/orders.atto", package:orders_pkg, source:orders },
      LinkUnit { path:"lib/math.atto", package:math_pkg, source:math },
    ]
}
fn reject(units: &[LinkUnit<'_>], code: &str) {
    let diag = compile_linked("src/main.atto", units).expect_err("invalid graph emitted guest");
    assert_eq!(diag.code, code, "unexpected diagnostic: {diag}");
}
#[test]
fn multi_hop_and_root_import_compile_deterministically() {
    let units = fixture(MAIN, ORDERS, MATH, "app", "vendor", "other");
    let binary = compile_linked("src/main.atto", &units).expect("three-file guest");
    assert!(binary.starts_with(b"\0asm\x01\0\0\0"));
    let reversed = [units[2], units[1], units[0]];
    assert_eq!(binary, compile_linked("src/main.atto", &reversed).unwrap());
}
#[test]
fn immediate_importer_not_entry_must_have_pkg_visibility() {
    let units = fixture(MAIN, ORDERS, PKG_MATH, "mathlib", "orderslib", "mathlib");
    reject(&units, "E_VISIBILITY");
    let allowed = fixture(MAIN, ORDERS, PKG_MATH, "app", "mathlib", "mathlib");
    assert!(compile_linked("src/main.atto", &allowed).is_ok());
}
#[test]
fn deep_private_member_access_does_not_leak() {
    let b = ORDERS.replace("Math.double(x)", "Math.secret()");
    reject(&fixture(MAIN, &b, MATH, "app", "app", "app"), "E_PRIVATE");
}
#[test]
fn missing_deep_source_must_fail_closed() {
    let units = fixture(MAIN, ORDERS, MATH, "app", "app", "app");
    reject(&units[..2], "E_UNBOUND");
}
#[test]
fn transitive_cycle_is_not_a_valid_acyclic_dependency_graph() {
    let c = MATH.replace("export pub module Math;", "import module Orders from \"./orders.atto\";\nexport pub module Math;");
    reject(&fixture(MAIN, ORDERS, &c, "app", "app", "app"), "E_IMPORT_CYCLE");
}
#[test]
fn alternate_paths_and_duplicates_do_not_forge_same_file_identity() {
    let c = MATH.replace("export pub module Math;", "export pub module Math;");
    let b = ORDERS.replace("./math.atto", "+lib/math.atto");
    assert!(compile_linked("src/main.atto",&fixture(MAIN,&b,&c,"app","app","app")).is_ok());
    let traversal = ORDERS.replace("./math.atto","./../math.atto");
    reject(&fixture(MAIN,&traversal,MATH,"app","app","app"),"E_IMPORT_PATH");
}
#[test]
fn duplicate_module_name_in_unrelated_paths_is_rejected() {
    let one = "import module Math from \"+lib/math.atto\";\n";
    let two = "import module Math from \"+lib/alternate.atto\";\n";
    let main = MAIN.replace("pub fnc main(): int", &(one.to_string()+two+"pub fnc main(): int"));
    let extra = LinkUnit {path:"lib/alternate.atto",package:"app",source:MATH};
    let mut units=fixture(&main, ORDERS, MATH,"app","app","app").to_vec();
    units.push(extra);
    reject(&units,"E_DUPLICATE");
}
#[test]
fn nested_reexport_remains_rejected_after_transitive_linking() {
    let b = ORDERS.replace("export pub module Orders;", "export pub module Math;\nexport pub module Orders;");
    reject(&fixture(MAIN,&b,MATH,"app","app","app"),"E_UNSUPPORTED");
}
#[test]
fn namespace_in_middle_of_dependency_never_passes_as_header() {
    let b = ORDERS.replace("define module Orders", "use namespace stolen;\ndefine module Orders");
    reject(&fixture(MAIN,&b,MATH,"app","app","app"),"E_SYNTAX");
}

#[test]
fn imports_are_not_ambient_authority_across_sibling_source_files() {
    // main imports both Math and Orders, but Orders has no Math import.
    // The old flattening approach would accidentally make Math available
    // to Orders via the entry's import, bypassing direct importer rules.
    let main = MAIN.replace("pub fnc main(): int",
        "import module Math from \"+lib/math.atto\";\npub fnc main(): int");
    let orders = ORDERS.replace("import module Math from \"./math.atto\";", "");
    reject(&fixture(&main, &orders, MATH, "app", "app", "mathlib"), "E_VISIBILITY");
}

#[test]
fn imported_module_cannot_capture_private_entry_file_functions() {
    let main = MAIN.replace("pub fnc main(): int",
        "fnc private_helper(): int { return 42; }\npub fnc main(): int");
    let orders = ORDERS.replace("Math.double(x)", "private_helper()");
    reject(&fixture(&main, &orders, MATH, "app", "app", "app"), "E_VISIBILITY");
}

#[test]
fn declaring_a_member_named_main_is_not_equivalent_to_calling_entry_main() {
    let orders = ORDERS.replace("pub fnc total(int x): int",
        "pub fnc main(): int { return 7; }\n    pub fnc total(int x): int");
    assert!(compile_linked("src/main.atto",
        &fixture(MAIN, &orders, MATH, "app", "app", "app")).is_ok());
}


#[test]
fn linker_rejects_user_authored_compiler_qualified_symbol_names() {
    let malicious=ORDERS.replace("pub fnc total(int x): int",
        "fnc __atto_q_untrusted(): int { return 1; }\n    pub fnc total(int x): int");
    reject(&fixture(MAIN,&malicious,MATH,"app","app","app"),"E_RESERVED");
}

#[test]
fn qualified_id_namespace_distinguishes_same_leaf_without_source_disclosure() {
    use atto_compiler::linked_symbols::QualifiedModuleId;
    let a=QualifiedModuleId::new("billing@1.0","billing","lib/math.atto","Math").unwrap();
    let b=QualifiedModuleId::new("analytics@1.0","analytics","lib/math.atto","Math").unwrap();
    assert_ne!(a.internal_name(),b.internal_name());
    assert!(a.internal_name().starts_with("__atto_q_"));
    // Guest callers cannot choose the internal name to bypass export checks.
    let forged=MAIN.replace("Orders.total(21)",
        "__atto_q_untrusted.total(21)");
    reject(&fixture(&forged,ORDERS,MATH,"app","app","app"),"E_RESERVED");
}
