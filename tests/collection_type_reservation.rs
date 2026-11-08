//! Prevent user-defined type/function declarations from impersonating the
//! canonical generic collections, Option/Result or intrinsic borrow capabilities.
//! This is deliberately *not* an implementation of their runtime semantics.

use atto_compiler::compile;

const RESERVED: &[&str] = &[
    "Option", "Result", "Array", "FixedArray", "List", "FixedList",
    "Vector", "Slice", "Tuple", "Map", "Set", "ReadLoan", "WriteLoan",
];

#[test]
fn class_declarations_cannot_forge_core_collection_or_loan_identity() {
    for name in RESERVED {
        let source = format!(
            "define class {name} as\n  mut int n = 0;\nend\n\npub fnc main(): int {{ return 42; }}\n"
        );
        let error = compile(&source).expect_err(name);
        assert_eq!(error.code, "E_DUPLICATE", "{name}: {error}");
    }
}

#[test]
fn plain_functions_cannot_masquerade_as_core_type_constructors() {
    for name in RESERVED {
        let source = format!(
            "fnc {name}(): int {{ return 42; }}\npub fnc main(): int {{ return 42; }}\n"
        );
        let error = compile(&source).expect_err(name);
        assert_eq!(error.code, "E_DUPLICATE", "{name}: {error}");
    }
}

#[test]
fn reserved_core_names_do_not_change_user_types_or_native_wasm() {
    let source = include_str!("fixtures/pass/50-class-constructor-method.atto");
    let bytes = compile(source).expect("ordinary user class compiles");
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn array_and_option_types_still_fail_closed_instead_of_being_faked() {
    for type_name in ["Option<int>", "Array<int>", "Vector<int>", "ReadLoan<int>"] {
        let source = format!("pub fnc main(): int {{ val value: {type_name} = 0; return 42; }}");
        assert!(compile(&source).is_err(), "{type_name} was unexpectedly admitted");
    }
}
