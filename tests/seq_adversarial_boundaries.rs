//! Adversarial seq admission. These tests deliberately exercise compiler diagnostics
//! as well as successful compilation; they do not claim cross-backend execution.
//! Layered on atto-source.rs #330's v1/v2 integration candidate.
use atto_compiler::{
    ast::{ExprKind, Stmt},
    compile,
    lexer::lex,
    parser::Parser,
};

fn compiles(source: &str) {
    let first = compile(source).unwrap_or_else(|err| panic!("admission failed: {err}\n{source}"));
    assert!(first.starts_with(b"\0asm\x01\0\0\0"), "invalid Wasm header: {source}");
    assert_eq!(first, compile(source).unwrap(), "unstable byte output: {source}");
}

fn rejects(source: &str) {
    assert!(compile(source).is_err(), "incorrectly admitted: {source}");
}

#[test]
fn seq_singleton_nesting_precedence_and_shadowing() {
    for source in [
        "pub fnc main(): int { return seq(42); }",
        "pub fnc main(): int { return seq(1, 2) + 40; }",
        "pub fnc main(): int { return seq(1, seq(2, 42)); }",
        "pub fnc main(): int { return (> 1, seq(2, 42)); }",
        "pub fnc main(): int { return seq(1, (> 2, 42)); }",
        "pub fnc main(): int { return seq(\"literal,comma\", 42); }",
        "pub fnc main(): int { return seq(/* before */ 1 /* after */, 42); }",
        "pub fnc main(): int { let int n = 9; return seq(let int n = 41, n + 1); }",
        "pub fnc main(): int { let int x = 0; return seq(x += 1, seq(x += 1, x * 21)); }",
        "pub fnc main(): int { const int a = seq(let int x = 20, x + 1); const int b = seq(let int x = 19, x + 2); return a + b; }",
        "pub fnc main(): int { seq 10, 11; return 42; }",
        "pure fnc p(): int { return seq(1, 42); } pub fnc main(): int { return p(); }",
    ] {
        compiles(source);
    }
}

#[test]
fn call_argument_comma_ownership_is_syntactic_and_explicit() {
    let source = "fnc id(int v): int { return v; } pub fnc main(): int { return id(seq 1, id(2), 42); }";
    let program = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    let main = program.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Return(Some(value), _) = &main.body[0] else { panic!("expected main return") };
    let ExprKind::Call(_, args) = &value.kind else { panic!("expected call") };
    assert_eq!(args.len(), 1, "bare seq must own all commas through the matching call ')'");
    let ExprKind::Sequence(steps, tail) = &args[0].kind else { panic!("expected single sequence argument") };
    assert_eq!(steps.len(), 2);
    assert!(matches!(&tail.kind, ExprKind::Int(Some(42))));

    for source in [
        "fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add(1, seq 0, 41); }",
        "fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add(seq(1, 2), 40); }",
        "fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add((> 1, 2), 40); }",
        "fnc id(int v): int { return v; } pub fnc main(): int { return id(seq(1, id(2), 42)); }",
    ] {
        compiles(source);
    }
}

#[test]
fn seq_rejects_empty_items_delimiters_nonvalues_and_bad_targets() {
    for source in [
        "pub fnc main(): int { return seq(); }",
        "pub fnc main(): int { return seq(, 42); }",
        "pub fnc main(): int { return seq(1,, 42); }",
        "pub fnc main(): int { return seq(1, 42,); }",
        "pub fnc main(): int { return seq(1; 42); }",
        "pub fnc main(): int { return seq(1, 42; }",
        "pub fnc main(): int { return seq(let int n = 42); }",
        "pub fnc main(): int { let int n = 1; return seq(n += 1); }",
        "pub fnc main(): int { let int n = 1; return seq(1, n = 42); }",
        "pub fnc main(): int { return seq(1, let int n = 42); }",
        "pub fnc main(): int { let int n = 0; return seq(1 + n = 42, n); }",
        "pub fnc main(): int { return seq(1, ^ 42); }",
        "pub fnc main(): int { return seq(return 1, 42); }",
        "pub fnc main(): int { return seq(1, (2, 3), 42); }",
    ] {
        rejects(source);
    }
}

#[test]
fn seq_does_not_bypass_types_scopes_mutability_ownership_or_effects() {
    for source in [
        "pub fnc main(): int { const int n = 1; return seq(n += 1, n); }",
        "pub fnc main(): int { return seq(let int n = 1, let int n = 2, n); }",
        "pub fnc main(): int { const int n = seq(let int hidden = 42, hidden); return hidden; }",
        "pub fnc main(): int { let int n = 1; return seq(n += 1, n) + hidden; }",
        "pub fnc main(): int { const int n = 42; return seq(take! n, n); }",
        "pure fnc p(): int { return seq(stdio.println(1), 42); } pub fnc main(): int { return p(); }",
        "fnc p(): bool { return seq(1, 42); } pub fnc main(): int { return 42; }",
    ] {
        rejects(source);
    }
}
