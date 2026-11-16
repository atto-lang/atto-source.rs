//! Scoped comma sequences: actual Rust compiler parse -> checker -> Wasm admission.
use atto_compiler::{compile, ast::{ExprKind, Stmt}, lexer::{lex, Tok}, parser::Parser};

fn accepted(source: &str) {
    let a = compile(source).unwrap_or_else(|e| panic!("unexpected compile rejection: {source}\n{e}"));
    assert_eq!(&a[..8], b"\0asm\x01\0\0\0", "missing Wasm magic");
    assert_eq!(a, compile(source).unwrap(), "non-deterministic Wasm output");
}

#[test]
fn nested_sequence_and_caret_return_compile() {
    for src in [
        "pub fnc main(): int { const int x = 5; const int y = x + 3; ^ seq let z = x + y, z + 4; }",
        "pub fnc main(): int { const int result = (> let x = 5, (> let y = x + 3, y)); ^ result + 34; }",
        "pub fnc main(): int { ^ (> let x = 5, let y = x * 8, y + 2); }",
        "pub fnc main(): int { ^ begin let x = 5, let y = x * 8, y + 2 end; }",
        "pub fnc main(): int { seq let a = 4, a * 10; ^ 42; }",
        "fnc add(int a, int b): int { ^ a + b; } pub fnc main(): int { ^ add((> let x = 2, x), 40); }",
        "pub fnc main(): int { ^ seq (> let a = 1, a * 20), 42; }",
        "pub fnc main(): int { ^(> let x = 5, x + 37); }",
    ] { accepted(src); }
}

#[test]
fn nested_sequence_has_its_own_ast_and_lexer_marker() {
    let src = "pub fnc main(): int { const int x = (> let a = 5, a + 3); ^ x; }";
    let toks = lex(src).unwrap();
    assert!(toks.iter().any(|t| t.tok == Tok::SequenceOpen));
    let ast = Parser::new(toks).parse_program().unwrap();
    let Stmt::Bind { init, .. } = &ast.funcs[0].body[0] else { panic!("expected binding"); };
    assert!(matches!(init.kind, ExprKind::Sequence(_, _)));
}

#[test]
fn sequence_syntax_and_scopes_fail_closed() {
    for src in [
        "pub fnc main(): int { ^ (>); }",
        "pub fnc main(): int { ^ (> let x = 5, x + 3,); }",
        "pub fnc main(): int { ^ (> let x = 5); }",
        "pub fnc main(): int { ^ seq let x = 5, let y = 8; }",
        "pub fnc main(): int { ^ (> let x = 5, let x = 6, x); }",
        "pub fnc main(): int { const int x = (> let y = 5, y + 3); ^ y; }",
        "pub fnc main(): int { ^ (> let x = 5, x + 3; }",
        "pub fnc main(): int { seq let x = 5, x + 37; }",
        "pub fnc main(): int { ^ (> let x = 2, x + 3) ^ 4; }",
        "fnc seq(int x): int { return x; } pub fnc main(): int { ^ seq(42); }",
        "pub fnc main(): int { ^ begin(42); }",
    ] {
        assert!(compile(src).is_err(), "malformed or untyped sequence compiled: {src}");
    }
}

#[test]
fn expression_delimiters_stay_distinct() {
    accepted("pub fnc main(): int { if (7 > 4) then ^ 42; else ^ 0; fi }");
    assert!(compile("pub fnc main(): int { ^ (1, 2); }").is_err(),
        "ordinary parentheses must never become a comma sequence");
    assert!(lex("pub fnc main(): int { ^ 1 ^^ 0; }").is_err(),
        "reserved XOR must not lex as two caret returns");
}

#[test]
fn pure_sequence_checks_every_step_and_tail() {
    accepted("pure fnc f(): int { ^ (> let x = 5, x + 3); } pub fnc main(): int { ^ f(); }");
    for src in [
        "pure fnc f(): int { ^ (> stdio.println(5), 42); } pub fnc main(): int { ^ f(); }",
        "pure fnc f(): int { ^ begin stdio.println(5), 42 end; } pub fnc main(): int { ^ f(); }",
        "pure fnc f(): int { ^ seq stdio.println(5), 42; } pub fnc main(): int { ^ f(); }",
    ] {
        assert!(compile(src).is_err(), "effectful operation hidden inside pure sequence: {src}");
    }
}
