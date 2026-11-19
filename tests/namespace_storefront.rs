//! Regression contract: a namespace header must compose with a lowercase,
//! explicitly imported relative module and a typed stdio.println call.
use atto_compiler::linker::{compile_linked, LinkUnit};

const ENTRY: &str = r#"use namespace storefront;
import module math from "./math.atto";

pub fnc main(): int {
  stdio.println(math.square(7)); // 49, if the companion module is available
  return 0;
}"#;

const MATH: &str = r#"use namespace arithmetic;
define pub module math as
    pub fnc square(int x): int { return x * x; }
end"#;

fn compile(entry: &str, dependency: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_linked(
        "src/main.atto",
        &[
            LinkUnit {
                path: "src/main.atto",
                package: "storefront@1.0.0",
                source: entry,
            },
            LinkUnit {
                path: "src/math.atto",
                package: "storefront@1.0.0",
                source: dependency,
            },
        ],
    )
}

#[test]
fn storefront_namespace_relative_lowercase_math_and_println_compile() {
    let wasm = compile(ENTRY, MATH).expect("namespace + linked math.square + println");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(ENTRY, MATH).expect("deterministic linked binary"));
}

#[test]
fn namespace_header_is_not_an_import_or_export_authority() {
    let missing = compile_linked(
        "src/main.atto",
        &[LinkUnit {
            path: "src/main.atto",
            package: "storefront@1.0.0",
            source: ENTRY,
        }],
    )
    .expect_err("namespace cannot supply the missing math file");
    assert_eq!(missing.code, "E_UNBOUND");

    let hidden = MATH.replace("define pub module math", "define module math");
    let private = compile(ENTRY, &hidden).expect_err("private module cannot be imported");
    assert_eq!(private.code, "E_VISIBILITY");
}

#[test]
fn imported_module_and_member_names_are_case_sensitive_and_public() {
    let bad_case = ENTRY.replace("import module math", "import module Math");
    let err = compile(&bad_case, MATH).expect_err("module name is case-sensitive");
    assert_eq!(err.code, "E_UNBOUND");

    let hidden_square = MATH.replace("pub fnc square", "fnc square");
    let err = compile(ENTRY, &hidden_square).expect_err("private function must not leak");
    assert_eq!(err.code, "E_PRIVATE");
}

#[test]
fn explicit_dotted_and_folder_derived_namespace_headers_link() {
    let dotted = ENTRY.replace("use namespace storefront;", "use namespace shop.frontend;");
    assert!(compile(&dotted, MATH).is_ok(), "dotted explicit namespace must link");

    let derived = ENTRY.replace("use namespace storefront;", "use default namespace;");
    assert!(compile(&derived, MATH).is_ok(), "containing folder namespace must link");

    let parent = ENTRY.replace("use namespace storefront;", "use default namespace(2);");
    let linked = compile_linked("shop/src/main.atto", &[
        LinkUnit { path: "shop/src/main.atto", package: "storefront@1.0.0", source: &parent },
        LinkUnit { path: "shop/src/math.atto", package: "storefront@1.0.0", source: MATH },
    ]);
    assert!(linked.is_ok(), "two-component derived namespace must link: {linked:?}");
}

#[test]
fn standalone_use_identifier_does_not_become_a_namespace_header() {
    // "use namespace" is a source header, but "use" alone is not reserved.
    // A valid local name inside main must not be rejected by header scanning.
    let with_use = ENTRY.replace("return 0;", "let int use = 0; return use;");
    let linked = compile(&with_use, MATH);
    assert!(linked.is_ok(), "local identifier named use must link: {linked:?}");
}

#[test]
fn missing_default_namespace_depth_and_misplaced_headers_fail_closed() {
    let too_deep = ENTRY.replace("use namespace storefront;", "use default namespace(2);");
    let diagnostic = compile(&too_deep, MATH).expect_err("src/main.atto has only one parent folder");
    assert_eq!(diagnostic.code, "E_NAMESPACE_DEPTH");

    let duplicated = ENTRY.replace("use namespace storefront;",
        "use namespace storefront;\nuse namespace stolen;");
    let diagnostic = compile(&duplicated, MATH).expect_err("a file has at most one namespace");
    assert_eq!(diagnostic.code, "E_DUPLICATE");

    let misplaced = ENTRY.replace("use namespace storefront;\nimport module math from \"./math.atto\";",
        "import module math from \"./math.atto\";\nuse namespace storefront;");
    let diagnostic = compile(&misplaced, MATH).expect_err("namespace header must precede import");
    assert_eq!(diagnostic.code, "E_SYNTAX");
}

#[test]
fn explicit_module_export_is_linked_and_stripped_without_reexport() {
    let exported = MATH.replace(
        "define pub module math",
        "export pub module math;\ndefine module math",
    );
    let wasm = compile(ENTRY, &exported)
        .expect("export pub module is an equivalent import authority");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));

    let pkg_only = exported.replace(
        "export pub module math;",
        "export pub[pkg] module math;",
    );
    assert!(compile(ENTRY, &pkg_only).is_ok(),
        "same-package import may see pub[pkg] export");

    let foreign = compile_linked(
        "src/main.atto",
        &[
            LinkUnit {
                path: "src/main.atto",
                package: "shop@1.0.0",
                source: ENTRY,
            },
            LinkUnit {
                path: "src/math.atto",
                package: "storefront@1.0.0",
                source: &pkg_only,
            },
        ],
    ).expect_err("package-private modules do not cross package boundaries");
    assert_eq!(foreign.code, "E_VISIBILITY");
}
