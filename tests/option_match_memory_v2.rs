//! Check real compiler memory admission for unused ADT readers.
use atto_compiler::{compile, check, lexer::lex, parser::Parser};
#[test]
fn unused_option_match_requires_memory() {
    for formal in ["int? x", "x: int?", "Option<int> x"] {
        let source = format!("fnc inspect({formal}): int {{ match x over on None -> {{ return 0; }} on Some(n) -> {{ return n; }} end }} pub fnc main(): int {{ return 42; }}");
        let p = Parser::new(lex(&source).unwrap()).parse_program().unwrap();
        let typed = check::check(&p).unwrap();
        assert!(typed.heap, "uncalled ADT reader requires memory: {formal}");
        assert!(compile(&source).is_ok());
    }
}
#[test]
fn scalar_only_does_not_acquire_memory() {
    let source = "pub fnc main(): int { return 42; }";
    let p = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert!(!check::check(&p).unwrap().heap);
}
