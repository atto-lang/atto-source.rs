//! Compiler-backed fail-closed checks on the actual Cranelift scalar MIR.
//! These DO NOT claim that Array, Option or collection borrowing is compiled.

const RESERVED: [&str; 13] = [
    "Option", "Result", "Array", "FixedArray", "Vector", "List",
    "FixedList", "Slice", "Tuple", "Map", "Set", "ReadLoan", "WriteLoan",
];

#[test]
fn canonical_collection_name_spoofing_never_emits_a_native_object() {
    for name in RESERVED {
        let source = format!(
            "fnc {name}(): int {{ return 42; }} pub fnc main(): int {{ return {name}(); }}"
        );
        let err = oreslang_source_rs::compile(&source).expect_err(name);
        assert!(err.contains("reserved_collection_name"), "{name}: {err}");
        assert!(oreslang_source_rs::compile_checked_i64(&source).is_err(), "{name}");
        assert!(oreslang_source_rs::compile_with_effect_ir(&source).is_err(), "{name}");
    }
}

#[test]
fn unsupported_option_and_collection_operators_remain_unsupported() {
    let unsupported = [
        "pub fnc main(): int { const x = [1,2]; return x[0]; }",
        "pub fnc main(): int { return Array.get(0); }",
        "pub fnc main(): int { return Some(42); }",
    ];
    for source in unsupported {
        assert!(oreslang_source_rs::compile(source).is_err(),
            "unproven collection syntax must not emit native bytes");
    }
}

#[test]
fn unrelated_scalar_functions_remain_native_objects() {
    let bytes = oreslang_source_rs::compile(
        "fnc helper(): int { return 42; } pub fnc main(): int { return helper(); }"
    ).expect("ordinary native scalar function");
    assert!(!bytes.is_empty());
}
