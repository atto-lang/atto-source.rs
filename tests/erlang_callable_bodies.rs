use atto_compiler::{compile, lexer::lex, parser::Parser, ast::Stmt};

#[test]
fn five_body_spellings_parse_and_compile() {
    let src = r#"
        fnc one(): int { return 5; }
        fnc two(): int -> { return 5; }
        fnc three() -> int { return 5; }
        fnc four() -> int
            let int x = 3,
            x + 2
        end
        fnc five() -> int ::
            let int x = 3,
            x + 2
        end
        pub fnc main(): int { return one() + two() + three() + four() + five(); }
    "#;
    let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(p.funcs.len(), 6);
    for i in [3, 4] {
        assert!(matches!(p.funcs[i].body.last(), Some(Stmt::Return(Some(_), _))));
    }
    let first = compile(src).unwrap();
    assert_eq!(&first[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    assert_eq!(first, compile(src).unwrap(), "deterministic guest bytes");
}

#[test]
fn routine_and_member_grammar_use_same_body_rules() {
    let src = r#"
        routine direct() -> int :: let int x = 3, x + 2 end
        define class Sample as
            fnc inc() -> int :: let int x = 4, x + 1 end
            fnc boxed(): int -> { return 5; }
        end
        pub fnc main(): int { return direct(); }
    "#;
    let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(p.classes.len(), 1);
    assert_eq!(p.classes[0].methods.len(), 2);
    assert!(matches!(p.classes[0].methods[0].body.last(), Some(Stmt::Return(Some(_), _))));
    assert!(compile(src).is_ok());
}

#[test]
fn parser_keeps_ordinary_c_body_returns_explicit() {
    for src in [
        "pub fnc main(): int { 5; }",
        "pub fnc main() -> int { 5; }"
    ] {
        let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
        assert!(matches!(p.funcs[0].body.last(), Some(Stmt::Expr(_))));
        assert!(compile(src).is_err(), "C-style implicit return must remain invalid");
    }
}

#[test]
fn erlang_bodies_reject_ambiguous_or_missing_delimiters() {
    let cases = [
        ("pub fnc main() -> int :: 3, 5, end", "E_SYNTAX"),
        ("pub fnc main() -> int :: 3; 5 end", "E_SYNTAX"),
        ("pub fnc main() -> int :: 3 5 end", "E_SYNTAX"),
        ("pub fnc main() -> int :: 3\n5 end", "E_SYNTAX"),
        ("pub fnc main(): int :: 5 end", "E_SYNTAX"),
        ("pub fnc main(): int -> 5 end", "E_SYNTAX"),
        ("pub fnc main() -> int :: 3, 5", "E_EOF"),
        ("pub fnc main() -> int :: , 5 end", "E_SYNTAX"),
    ];
    for (src, code) in cases {
        let diagnostic = compile(src).expect_err(src);
        assert_eq!(diagnostic.code, code, "{src}: {diagnostic}");
    }
}

#[test]
fn erlang_tail_preserves_nested_argument_commas_and_return_typing() {
    let good = r#"
        fnc add(int a, int b): int { return a + b; }
        pub fnc main() -> int ::
            let int x = 2,
            add(x, 3)
        end
    "#;
    assert!(compile(good).is_ok());
    let mismatch = "pub fnc main() -> bool :: 5 end";
    assert!(compile(mismatch).is_err(), "tail must still be type-checked");
    let no_tail = "pub fnc main() -> int :: let int x = 5 end";
    assert!(compile(no_tail).is_err(), "non-void cannot fall through");
}
