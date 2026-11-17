//! Equality operator admission in the real lexer/parser/checker/Wasm backend.
use atto_compiler::{compile, ast::{BinOp, ExprKind, Stmt}, lexer, parser::Parser};

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
    // No user-defined equals method: no pointer-identity fallback.
    assert_eq!(compile(unsupported).unwrap_err().code, "E_EQUALS");
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

#[test]
fn parser_retains_value_and_identity_opcodes_without_aliasing() {
    for (spelling, expected) in [
        ("eq", BinOp::Eq),
        ("neq", BinOp::Ne),
        ("!eq", BinOp::Ne),
        ("==", BinOp::IdEq),
        ("!=", BinOp::IdNe),
    ] {
        let source = format!("fnc compare(): bool {{ return 1 {spelling} 2; }} pub fnc main(): int {{ return 0; }}");
        let ast = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
        let Stmt::Return(Some(expr), _) = &ast.funcs[0].body[0] else {
            panic!("{spelling}: expected return expression");
        };
        let ExprKind::Binary(actual, _, _) = &expr.kind else {
            panic!("{spelling}: missing operator AST");
        };
        assert_eq!(*actual, expected, "{spelling}: wrong operator");
    }
}

#[test]
fn identity_rejects_strings_different_class_domains_and_tagged_values() {
    for src in [
        "pub fnc main(): int { if \"same\" == \"same\" then return 1; fi return 0; }".to_string(),
        "pub fnc main(): int { if \"same\" != \"other\" then return 1; fi return 0; }".to_string(),
        "define class A as end define class B as end pub fnc main(): int { val a = new A(); val b = new B(); if a == b then return 1; fi return 0; }".to_string(),
    ] {
        let d = compile(&src).expect_err("identity must never lower to scalar or mismatched-object equality");
        assert_eq!(d.code, "E_IDENTITY_TYPE", "{src}: {d}");
        assert!(d.pos.line >= 1 && d.pos.col >= 1, "{d}");
    }
}

#[test]
fn generator_handles_are_not_value_or_reference_equal() {
    // An affine Generator<int> uses an i64 continuation handle. Reject *before*
    // emitting an i32.eq guest opcode or leaking an unclosed generator.
    for spelling in ["eq", "neq", "==", "!="] {
        let src = format!(r#"
            fnc *gen values(): Generator<int> {{ yield 1; }}
            pub fnc main(): int {{
                if values() {spelling} values() then return 1; fi
                return 0;
            }}
        "#);
        let d = compile(&src).expect_err("opaque generator comparison must not emit a guest");
        assert_eq!(
            d.code,
            if matches!(spelling, "==" | "!=") { "E_IDENTITY_TYPE" } else { "E_UNSUPPORTED" },
            "{spelling}: {d}"
        );
    }
}

#[test]
fn checked_equals_dispatch_preserves_distinct_identity_and_ownership() {
    let source = include_str!("fixtures/pass/226-checked-object-equals.atto");
    let wasm = compile(source).expect("checked non-moving equals method must compile");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(source).unwrap(), "exact guest compiler deterministic");
}

#[test]
fn equals_signature_validation_is_fail_closed_even_when_unused() {
    for declaration in [
        "pub equals(int other): bool { return true; }",
        "pub equals(Point other): int { return 1; }",
        "equals(Point other): bool { return true; }",
        "pub static fnc equals(Point other): bool { return true; }",
        "pub equals(): bool { return true; }",
    ] {
        let source = format!("define class Point as {declaration} end pub fnc main(): int {{ return 0; }}");
        let err = compile(&source).expect_err(&source);
        assert_eq!(err.code, "E_EQUALS_SIGNATURE", "{declaration}: {err}");
    }
}

#[test]
fn equality_borrows_both_objects_and_rejects_transferring_other_parameter() {
    for transfer in [
        "val stolen = other;",
        "val stolen = take! other;",
        "sink(other);",
    ] {
        let src = format!(r#"
            define class Point as
                pub final int n: 41;
                pub equals(Point other): bool {{
                    {transfer}
                    return self.n eq other.n;
                }}
            end
            fnc sink(Point p): void {{}}
            pub fnc main(): int {{ return 0; }}
        "#);
        let d = compile(&src).expect_err(&src);
        assert_eq!(d.code, "E_BORROW", "{transfer}: {d}");
    }
}

#[test]
fn malformed_or_missing_equals_never_uses_identity_as_a_fallback() {
    for op in ["eq", "neq"] {
        let src = format!(r#"
            define class Point as pub final int n: 41; end
            pub fnc main(): int {{
                val a = new Point();
                val b = new Point();
                if a {op} b then return 1; fi
                return 0;
            }}
        "#);
        let d = compile(&src).expect_err(&src);
        assert_eq!(d.code, "E_EQUALS", "{op}: {d}");
    }
}

#[test]
fn explicit_equals_calls_borrow_other_without_implicit_moves() {
    let src = r#"
        define class Point as
            pub final int n: 12;
            pub equals(Point other): bool { return self.n eq other.n; }
        end
        pub fnc main(): int {
            val a = new Point();
            val b = new Point();
            if a.equals(b), a eq b, b == b then return 42; fi
            return 0;
        }
    "#;
    let wasm = compile(src).expect("explicit method and operator equality must both borrow");
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");

    let wrong = r#"
        define class Point as
            pub equals(Point other): bool { return true; }
        end
        pub fnc main(): int {
            val a = new Point();
            if a.equals(42) then return 1; fi
            return 0;
        }
    "#;
    assert_eq!(compile(wrong).unwrap_err().code, "E_TYPE");
}
