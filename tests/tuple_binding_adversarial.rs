//! Independent adversarial edge-case checks for tuple ownership and value semantics.
//! The tests must execute against the exact compiler commit, not a host reference.
use atto_compiler::compile;

#[test]
fn zero_arity_tuple_through_real_function_is_well_defined() {
    let source = r#"
pub fnc zero(): Tuple<> { return (); }
pub fnc main(): int {
    let () as val unit = zero();
    return 7;
}
"#;
    let result = compile(source);
    assert!(result.is_ok(), "zero Tuple should be valid: {:?}", result.err());
}

#[test]
fn shared_loan_prevents_rebinding_the_let_tuple_root() {
    let source = r#"
pub fnc main(): int {
    let (a,b) as foo = (1,2);
    val hold = borrow! foo;
    foo = (3,4);
    return a+b;
}
"#;
    assert_eq!(compile(source).unwrap_err().code, "E_BORROW",
        "an outstanding shared borrow must block root rebinding");
}

#[test]
fn mutable_tuple_field_assignment_never_bypasses_val_or_const() {
    for root in ["as foo", "as val foo", "as const foo"] {
        let source = format!(r#"
pub fnc main(): int {{
    let (a,b) {root} = (1,2);
    foo.e0 = 42;
    return a;
}}
"#);
        let error = compile(&source).unwrap_err();
        assert!(matches!(error.code, "E_OWNERSHIP" | "E_IMMUTABLE" | "E_BORROW"),
            "{root}: unexpected diagnostic {:?}", error);
    }
}

#[test]
fn constexpr_tuple_root_read_respects_immutability() {
    let source = r#"
pub fnc main(): int {
    let (a, const b) as const root = (4,5);
    return root.e0+b;
}
"#;
    assert!(compile(source).is_ok());
    let reject = r#"
pub fnc main(): int {
    let (a,b) as const root = (4,5);
    root = (3,7);
    return a;
}
"#;
    assert_eq!(compile(reject).unwrap_err().code, "E_IMMUTABLE");
}

#[test]
fn unqualified_tuple_lhs_reports_syntax_not_runtime_unsupported() {
    let source = r#"
pub fnc fixtures(): Tuple<int,int,int> { return (1,2,3); }
pub fnc main(): int {
    (a, r, mut j) = fixtures();
    return 0;
}
"#;
    assert_eq!(compile(source).unwrap_err().code, "E_SYNTAX");
}

#[test]
fn tuple_gate_preserves_parenthesized_comparison_expressions() {
    let source = r#"
pub fnc main(): int {
    let a = 2;
    let b = 4;
    if (a < b) then
       return 1;
    fi
    return 0;
}
"#;
    assert!(compile(source).is_ok());
}
