use atto_compiler::{lexer::lex, tuple_binding_admission::inspect};

#[test]
fn typed_state_loop_headers_are_not_tuple_destructuring() {
    for source in [
        "return loop (val a: int = 0, val b: int = a) { break b; };",
        "return loop (val int a = 0, val int b = 1) { break b; };",
        "return loop (let a: int = 0) { break a; };",
        "for (let i: int = 0; i < 2; i++) {}",
    ] {
        assert!(inspect(&lex(source).unwrap()).unwrap().is_empty(), "{source}");
    }
}

#[test]
fn actual_tuple_binders_remain_checked() {
    let source = "(val a: int, val b: int) as val pair = (1, 2);";
    assert_eq!(inspect(&lex(source).unwrap()).unwrap().len(), 1);
    let malformed = "(val a: Unsupported, val b: int) as val pair = (1, 2);";
    assert!(inspect(&lex(malformed).unwrap()).is_err());
}
