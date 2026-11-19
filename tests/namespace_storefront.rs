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
