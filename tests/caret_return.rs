//! Executable caret-return parser and checker admission for the Rust browser compiler.
//! Do not confuse this with support for seq or (> ...), which are separately gated.
use atto_compiler::{compile, lexer::{lex, Tok}};

fn accepts(source: &str) {
    compile(source).unwrap_or_else(|e| panic!("expected a compiled guest: {e}\n{source}"));
}

#[test]
fn caret_returns_from_free_functions_and_nested_control_flow() {
    for src in [
        "pub fnc main(): int { ^ 42; }",
        "pub fnc main(): int { ^(40 + 2); }",
        "pub fnc main(): int { val n = 40; ^ n + 2; }",
        "fnc add(int n): int { ^ n + 2; } pub fnc main(): int { ^ add(40); }",
        "pub fnc main(): int { if true then ^ 42; else ^ 0; fi }",
        "pub fnc main(): int { ^ match 5 over on 5 -> 42, default -> 0, end; }",
        "pub fnc main(): int { ^ seq match 5 over on 5 -> 42, default -> 0, end; }",
        "define class C as pub answer(): int { ^ 42; } end pub fnc main(): int { const c = new C(); ^ c.answer(); }",
        "fnc noop(): void { ^; } pub fnc main(): int { ^ 42; }",
    ] { accepts(src); }
}

#[test]
fn lexer_distinguishes_caret_from_comparison_and_reserved_xor() {
    let toks = lex("^ (1 > 0);").expect("caret and comparison lex");
    assert!(matches!(toks[0].tok, Tok::Caret));
    assert!(toks.iter().any(|t| matches!(t.tok, Tok::Gt)));
    assert!(lex("^^").is_err(), "reserved XOR remains unsupported");
}

#[test]
fn caret_cannot_be_infix_or_an_embedded_expression() {
    for src in [
        "pub fnc main(): int { ^ 2 ^ 3; }",
        "pub fnc main(): int { val n = ^ 42; ^ n; }",
        "pub fnc main(): int { ^; }",
        "pub fnc main(): int { ^ true; }",
    ] {
        assert!(compile(src).is_err(), "invalid caret or return type unexpectedly compiled: {src}");
    }
}

#[test]
fn parser_recovers_after_rejection() {
    assert!(compile("pub fnc main(): int { ^ 2 ^ 3; }").is_err());
    accepts("pub fnc main(): int { ^ 42; }");
}
