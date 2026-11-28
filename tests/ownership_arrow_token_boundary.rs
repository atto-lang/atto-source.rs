//! Token separation prerequisite only; no ownership-arrow admission claim.
use atto_compiler::compile;

#[test]
fn callable_separators_keep_identical_wasm() {
    let arrow = compile("pub fnc int <- main() { return 42; }").expect("arrow signature");
    let bullet = compile("pub fnc int • main() { return 42; }").expect("bullet signature");
    assert_eq!(&arrow[..8], b"\0asm\x01\0\0\0");
    assert_eq!(arrow, bullet);
}

#[test]
fn bullet_is_not_a_binding_initializer() {
    for source in [
        "pub fnc main(): int { let b = 42; let a • b; return a; }",
        "pub fnc main(): int { let a = 0; let b = 42; set a • b; return a; }",
    ] {
        assert!(compile(source).is_err(), "accepted bullet in binding: {source}");
    }
}
