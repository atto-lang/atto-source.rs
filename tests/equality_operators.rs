//! Equality operator admission in the real lexer/parser/checker/Wasm backend.
use atto_compiler::compile;

fn main_with_condition(condition: &str) -> String {
    format!("pub fnc main(): int {{ if {condition} then return 1; fi return 0; }}")
}

#[test]
fn primitives_use_value_operators_only() {
    for condition in ["1 eq 1", "1 neq 2", "true eq true", "false neq true", "1.5 eq 1.5"] {
        let source = main_with_condition(condition);
        let wasm = compile(&source).unwrap_or_else(|d| panic!("{condition}: {d}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    }
    for (condition, suggestion) in [("1 == 1", "eq"), ("1 != 2", "neq"), ("true == true", "eq"), ("1.0 != 2.0", "neq"), ("1 eq 1.0", "")] {
        let source = main_with_condition(condition);
        let diagnostic = compile(&source).expect_err(&source);
        if suggestion.is_empty() {
            assert_eq!(diagnostic.code, "E_TYPE", "{diagnostic}");
        } else {
            assert_eq!(diagnostic.code, "E_IDENTITY_TYPE", "{diagnostic}");
            assert!(diagnostic.to_string().contains(suggestion), "{diagnostic}");
        }
    }
}

#[test]
fn object_identity_is_separate_from_object_value_equality() {
    let valid = r#"
        define class Thing as end
        pub fnc main(): int {
            val a = new Thing();
            val b = new Thing();
            // Ordinary assignment moves ownership; use the live reference itself.
            if a == a, a != b then return 7; fi
            return 0;
        }
    "#;
    let wasm = compile(valid).unwrap();
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(valid).unwrap(), "deterministic class identity lowering");

    let unsupported = r#"
        define class Thing as end
        pub fnc main(): int {
            val a = new Thing();
            val b = new Thing();
            if a eq b then return 7; fi
            return 0;
        }
    "#;
    // The subset deliberately rejects object equals() until virtual dispatch
    // and the value-equality method contract are implemented.
    assert_eq!(compile(unsupported).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn mixed_reference_and_scalar_identity_fails_closed() {
    let src = r#"
        define class Thing as end
        pub fnc main(): int {
            val a = new Thing();
            if a == 1 then return 1; fi
            return 0;
        }
    "#;
    assert_eq!(compile(src).unwrap_err().code, "E_IDENTITY_TYPE");
}
