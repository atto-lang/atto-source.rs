//! Accepted inline-method spelling matrix. Parser assertions deliberately
//! do not pretend that out-of-line receiver dispatch is executable.
use atto_compiler::{ast::Stmt, lexer::lex, parser::Parser};

#[test]
fn inline_methods_accept_five_body_styles_with_legacy_prefixes() {
    let source = r#"
        define class Calculator as
            fnc colon_braces(): int { return 5; }
            method colon_arrow(): int -> { return 5; }
            arrow_braces() -> int { return 5; }
            arrow_end() -> int
                let int x = 3,
                x + 2
            end
            arrow_delimited() -> int ::
                let int x = 3,
                x + 2
            end
            static fnc static_braces(): int { return 5; }
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.classes.len(), 1);
    let methods = &program.classes[0].methods;
    assert_eq!(methods.len(), 6);
    for method in methods {
        assert_eq!(method.ret, atto_compiler::ast::Type::Int);
        assert!(matches!(method.body.last(), Some(Stmt::Return(Some(_), _))));
    }
    assert!(!methods[0].is_static);
    assert!(methods[5].is_static);
}

#[test]
fn inline_actor_methods_share_the_five_body_styles() {
    let source = r#"
        define actor Worker as
            fnc colon_braces(): int { return 5; }
            method colon_arrow(): int -> { return 5; }
            arrow_braces() -> int { return 5; }
            arrow_end() -> int
                let int x = 3,
                x + 2
            end
            arrow_delimited() -> int :: let int x = 3, x + 2 end
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.actors.len(), 1);
    let methods = &program.actors[0].methods;
    assert_eq!(methods.len(), 5);
    for method in methods {
        assert!(matches!(method.body.last(), Some(Stmt::Return(Some(_), _))));
    }
}

#[test]
fn duplicate_arity_is_not_resolved_by_receiver_style() {
    let source = r#"
        define class Calculator as
            fnc get() -> int :: 5 end
            get() -> int { return 6; }
        end
        pub fnc main(): int { return 0; }
    "#;
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.classes[0].methods.len(), 2);
    // The semantic checker (not source spelling) must reject duplicate owners.
    let error = atto_compiler::compile(source).unwrap_err();
    assert_eq!(error.code, "E_DUPLICATE");
}

#[test]
fn empty_erlang_body_is_not_a_valid_value_or_void_method() {
    for header in ["empty() -> int :: end", "empty() -> void end"] {
        let src = format!("define class C as {header} end pub fnc main(): int {{ return 0; }}");
        let d = Parser::new(lex(&src).unwrap()).parse_program().unwrap_err();
        assert_eq!(d.code, "E_SYNTAX", "{src}: {d}");
    }
}
