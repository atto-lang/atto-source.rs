//! `-> Type ... end` is explicit-return statement mode;
 //! `-> Type :: ..., tail end` is implicit-return comma mode.
use atto_compiler::{compile, lexer::lex, parser::Parser, ast::Stmt};

fn compiles(source: &str) {
    let bytes = compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(bytes, compile(source).unwrap(), "Wasm must be deterministic");
}

#[test]
fn semicolon_end_mode_and_explicit_caret_return() {
    let src = r#"
        define class Foo as
            pub rec1() -> int
                const z: int = 5;
                ^ z + 3;
            end
            pub rec2() -> int ::
                const z: int = 5,
                z + 5
            end
        end
        pub fnc main(): int {
            const f = new Foo();
            ^ f.rec1() + f.rec2();
        }
    "#;
    let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert!(matches!(p.classes[0].methods[0].body.last(), Some(Stmt::Return(Some(_), _))));
    assert!(matches!(p.classes[0].methods[1].body.last(), Some(Stmt::Return(Some(_), _))));
    compiles(src);
}

#[test]
fn unmarked_end_does_not_implicitly_return_last_expression() {
    let src = "pub fnc main() -> int const z: int = 5; z + 5; end";
    let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert!(matches!(p.funcs[0].body.last(), Some(Stmt::Expr(_))));
    assert!(compile(src).is_err(), "unmarked final expression must not return");
}

#[test]
fn malformed_body_modes_rejected_without_guessing() {
    for src in [
        "pub fnc main() -> int const z: int = 5, z + 5 end",
        "pub fnc main() -> int :: const z: int = 5; z + 5 end",
        "pub fnc main() -> int :: ^ const z: int = 5, z + 5 end",
        "pub fnc main() -> int ^ const z: int = 5; end",
        "pub fnc main(): int :: 5 end",
        "pub fnc main() -> int :: { ^ 5; }",
    ] {
        assert!(compile(src).is_err(), "invalid body admitted: {src}");
    }
}

#[test]
fn explicit_return_and_comma_tail_work_for_free_callables() {
    compiles("fnc f() -> int const z: int = 5; ^ z + 3; end pub fnc main(): int { ^ f(); }");
    compiles("fnc f() -> int :: const z: int = 5, z + 5 end pub fnc main(): int { ^ f(); }");
}
