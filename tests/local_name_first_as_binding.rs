use atto_compiler::compile;

fn source(binding: &str) -> String {
    format!("define fnc int • main() {{ {binding} return x; }}")
}

#[test]
fn name_first_as_matches_prefix_and_colon_scalar_bindings() {
    for qualifier in ["const", "val", "let"] {
        let prefix = compile(&source(&format!("{qualifier} int x = 42;"))).unwrap();
        let colon = compile(&source(&format!("{qualifier} x :int = 42;"))).unwrap();
        let name_as = compile(&source(&format!("{qualifier} x as int = 42;"))).unwrap();
        assert_eq!(prefix, colon, "{qualifier}: colon");
        assert_eq!(prefix, name_as, "{qualifier}: name-first as");
        assert_eq!(&name_as[..4], b"\0asm");
    }
}

#[test]
fn name_first_as_does_not_bypass_type_or_initializer_checks() {
    for binding in ["const x as int = true;", "val x as = 42;",
                    "let x as int;", "const x as int : int = 42;"] {
        assert!(compile(&source(binding)).is_err(), "{binding}");
    }
}

#[test]
fn postfix_identity_cast_remains_an_expression() {
    assert!(compile(&source("const x = 42 as int;")).is_ok());
    assert!(compile(&source("const x = true as int;")).is_err());
}

#[test]
fn const_mut_and_val_emit_identical_fixed_scalar_bindings() {
    for declaration in ["int x = 42;", "x : int = 42;", "x as int = 42;", "x = 42;"] {
        let val = compile(&source(&format!("val {declaration}"))).unwrap();
        let alias = compile(&source(&format!("const mut {declaration}"))).unwrap();
        assert_eq!(val, alias, "{declaration}");
    }
    assert!(compile(&source("val mut int x = 42;")).is_err());
    assert!(compile("define fnc int • main() { const mut int x = 42; x = 7; return x; }").is_err());
}

#[test]
fn repeated_scalar_let_uses_previous_initializer_then_fresh_slot() {
    let shadow = "define fnc int • main() { let int x = 10; let x :int = x + 10; let x as int = x + 22; return x; }";
    let renamed = "define fnc int • main() { let int a = 10; let b :int = a + 10; let c as int = b + 22; return c; }";
    assert_eq!(compile(shadow).unwrap(), compile(renamed).unwrap());
    for declaration in ["const int x = 1; const int x = 42;",
        "int x = 1; const x as int = 42;", "val int x = 1; const mut int x = 42;"] {
        assert!(compile(&source(declaration)).is_err(), "{declaration}");
    }
}
