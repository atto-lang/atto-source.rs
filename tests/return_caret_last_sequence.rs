//! Contextual `return ^ (...)`: compiler admission and AST disambiguation.
//! These tests compile real guest Wasm; guest execution is a separate CI gate.
//! The parser reuses ExprKind::Sequence and the existing checked return path.
use atto_compiler::{
    ast::{ExprKind, Stmt},
    compile,
    lexer::lex,
    parser::Parser,
};

fn accepted(source: &str) {
    let bytes = compile(source).unwrap_or_else(|e| panic!("unexpected rejection: {e}\n{source}"));
    assert!(bytes.len() >= 8);
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(bytes, compile(source).unwrap(), "non-deterministic guest bytes");
}

fn rejected(source: &str) {
    assert!(compile(source).is_err(), "invalid return sequence compiled: {source}");
}

#[test]
fn last_value_return_compiles_as_checked_sequence() {
    for src in [
        "pub fnc main(): int { return ^ (42); }",
        "pub fnc main(): int { return ^ (1, 42); }",
        "pub fnc main(): int { return ^ (1, 2, 42); }",
        "pub fnc main(): int { return^(1,42); }",
        "pub fnc main(): int { return ^ (val x = 40, x + 2); }",
        "pub fnc main(): int { return ^ ((> let x = 40, x), 42); }",
        "pub fnc main(): int { return seq let x = 40, x + 2; }",
        "pub fnc main(): int { ^ seq let x = 40, x + 2; }",
        "pub fnc main(): int { ^ (> let x = 40, x + 2); }",
        "fnc f(int n): int { return ^ (n + 1, n + 2); } pub fnc main(): int { return f(40); }",
        "pub fnc main(): int { return ^ (40, (42)); }",
    ] {
        accepted(src);
    }
}

#[test]
fn contextual_selector_has_sequence_ast_not_tuple_or_nested_return() {
    let src = "pub fnc main(): int { return ^ (10, 20, 42); }";
    let ast = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    let Stmt::Return(Some(expr), _) = &ast.funcs[0].body[0] else {
        panic!("return ^ must be one return statement");
    };
    let ExprKind::Sequence(steps, tail) = &expr.kind else {
        panic!("return ^ must lower to checked Sequence AST, not a tuple");
    };
    assert_eq!(steps.len(), 2);
    assert!(matches!(&steps[0], Stmt::Expr(_)));
    assert!(matches!(&steps[1], Stmt::Expr(_)));
    assert!(matches!(&tail.kind, ExprKind::Int(Some(42))));
}

#[test]
fn empty_sequence_has_specific_diagnostic_and_parser_recovers() {
    for source in [
        "pub fnc main(): int { return ^ (); }",
        "pub fnc main(): int { return ^ (  ); }",
        "pub fnc main(): int { return^(); }",
    ] {
        let error = compile(source).unwrap_err();
        assert_eq!(error.code, "E_EMPTY_SEQUENCE", "{source}: {error}");
    }
    accepted("pub fnc main(): int { return ^ (1, 42); }");
}

#[test]
fn malformed_or_ambiguous_forms_fail_closed() {
    for src in [
        "pub fnc main(): int { return ^ (1,); }",
        "pub fnc main(): int { return ^ (,1); }",
        "pub fnc main(): int { return ^ (1,,2); }",
        "pub fnc main(): int { return ^ (1, 2; }",
        "pub fnc main(): int { return ^ 42; }",
        "pub fnc main(): int { return ^ seq 1, 42; }",
        "pub fnc main(): int { return ^ (1, 42) + 5; }",
        "pub fnc main(): int { return 1, 42; }",
        "pub fnc main(): int { val x = ^ (1, 42); return x; }",
        "pub fnc main(): int { return ^ (1, true); }",
        "pub fnc main(): int { return ^ (val x = 42); }",
    ] {
        rejected(src);
    }
}

#[test]
fn tuple_parentheses_never_become_a_last_value_sequence() {
    // Tuple values/types are still unsupported by the current scalar browser
    // compiler, but must never be silently interpreted as sequence values.
    for src in [
        "pub fnc main(): int { return (1, 42); }",
        "pub fnc main(): int { ^ (1, 42); }",
        "pub fnc main(): int { return ^ ((1, 42)); }",
        "pub fnc main(): int { return ^ (1, (2, 42)); }",
    ] {
        rejected(src);
    }
}

#[test]
fn preceding_effects_and_ownership_are_not_skipped() {
    // Every prefix is checked through the same sequence typing path as seq.
    rejected("pure fnc f(): int { return ^ (stdio.println(1), 42); } pub fnc main(): int { return f(); }");
    // Scoped bindings cannot escape the sequence.
    rejected("pub fnc main(): int { val n = (> val hidden = 42, hidden); return ^ (n, hidden); }");
}
