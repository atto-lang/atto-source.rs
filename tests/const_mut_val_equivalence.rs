use atto_compiler::compile;

#[test]
fn const_mut_matches_val_scalar_artifact() {
    let a = compile("pub fnc main(): int { const mut x = 42; return x; }").unwrap();
    let b = compile("pub fnc main(): int { val x = 42; return x; }").unwrap();
    assert_eq!(a, b);
    let a = compile("pub fnc main(): int { const mut int x = 42; return x; }").unwrap();
    let b = compile("pub fnc main(): int { val int x = 42; return x; }").unwrap();
    assert_eq!(a, b);
}

#[test]
fn const_mut_never_grants_rebinding() {
    for qualifier in ["const mut", "val"] {
        let source = format!("pub fnc main(): int {{ {qualifier} x = 42; x = 7; return x; }}");
        assert!(compile(&source).is_err(), "{source}");
    }
}

#[test]
fn duplicate_val_mut_is_not_an_alternate_authority() {
    let d = compile("pub fnc main(): int { val mut x = 42; return x; }").unwrap_err();
    assert_eq!(d.code, "E_SYNTAX");
}

#[test]
fn const_mut_and_val_owned_record_artifact_equivalence() {
    let a = compile(r#"pub fnc main(): int { const mut Record r={foo:1}; r.foo=42; return r.foo; }"#).unwrap();
    let b = compile(r#"pub fnc main(): int { val Record r={foo:1}; r.foo=42; return r.foo; }"#).unwrap();
    assert_eq!(a, b);
    for q in ["val", "const mut"] {
        let source = format!("pub fnc main(): int {{ {q} Record r={{foo:1}}; r={{foo:42}}; return r.foo; }}");
        assert!(compile(&source).is_err(), "{source}");
    }
}
