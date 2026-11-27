//! Capability diagnostic contract; not positive aggregate guest support.
use atto_compiler::{compile, lexer::lex, parser::Parser};

#[test]
fn aggregate_parameter_patterns_reject_as_unsupported_not_flattened() {
    for source in [
        "fnc total((a: int,b: int)): int { return a+b; }",
        "fnc total(()): int { return 7; }",
        "fnc total({x,y}: {x: int,y: int}): int { return x+y; }",
        "fnc total(Point {x,y}): int { return x+y; }",
        "define class C as total(Point {x,y}): int { return x+y; } end",
        "pub fnc main(): int { val f = |(a,b)| -> a+b; return 0; }",
        "pub fnc main(): int { val f = |{x,y}| -> x+y; return 0; }",
        "pub fnc main(): int { val f = |Point {x,y}| -> x+y; return 0; }",
    ] {
        let d = Parser::new(lex(source).unwrap()).parse_program().unwrap_err();
        assert_eq!(d.code, "E_UNSUPPORTED", "{source}: {d}");
        assert!(d.message.contains("callable aggregate parameter patterns"), "{source}: {d}");
        let d = compile(source).unwrap_err();
        assert_eq!(d.code, "E_UNSUPPORTED", "{source}: {d}");
    }
}

#[test]
fn ordinary_formals_and_lambdas_retain_their_existing_arity() {
    let source = "fnc total(int a,b: int): int { return a+b; } pub fnc main(): int { return total(5,8); }";
    let p = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(p.funcs[0].params.len(), 2);
    let wasm = compile(source).unwrap();
    assert_eq!(&wasm[..8], &[0,97,115,109,1,0,0,0]);
    let source = "pub fnc main(): int { val f = |a,b| -> a+b; return 0; }";
    assert!(Parser::new(lex(source).unwrap()).parse_program().is_ok());
}
