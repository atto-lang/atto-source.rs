//! Generator syntax admission and fail-closed Wasm boundary.
//! These tests prove parsed AST identity, not guest generator execution.
//! Do not turn the playground preview runnable until continuation lowering lands.
use atto_compiler::{ast::{ExprKind, Stmt, Type}, lexer::{lex, Tok}, parser::Parser};

const SAMPLE: &str = r#"
fnc *gen count_to(int limit): Generator<int> {
  for (let i = 1; i <= limit; i++) do
    yield i;
  done
}
fnc *gen count_with_zero(int limit): Generator<int> {
  yield 0;
  yield* count_to(limit);
}
pub fnc main(): int {
  let sum = 0;
  for value in count_with_zero(4) do
    sum += value;
  done
  stdio.println(sum);
  return sum;
}
"#;

fn parse(s: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(s).expect("lex")).parse_program().expect("parse")
}

#[test]
fn generator_declaration_and_delegation_have_distinct_ast_nodes() {
    let p = parse(SAMPLE);
    assert_eq!(p.funcs.len(), 3);
    assert_eq!(p.funcs[0].name, "count_to");
    assert!(p.funcs[0].is_generator);
    assert_eq!(p.funcs[0].ret, Type::GeneratorInt);
    assert!(matches!(&p.funcs[0].body[0], Stmt::For { body, .. }
        if matches!(&body[0], Stmt::Yield(..))));
    assert!(p.funcs[1].is_generator);
    assert!(matches!(&p.funcs[1].body[0], Stmt::Yield(..)));
    assert!(matches!(&p.funcs[1].body[1], Stmt::YieldFrom(expr, _)
        if matches!(&expr.kind, ExprKind::Call(name, args) if name == "count_to" && args.len() == 1)));
    assert!(!p.funcs[2].is_generator);
    assert!(matches!(&p.funcs[2].body[1], Stmt::ForIn { name, source, body, .. }
        if name == "value"
            && matches!(&source.kind, ExprKind::Call(f, _) if f == "count_with_zero")
            && !body.is_empty()));
}

#[test]
fn yield_star_is_contiguous_and_never_conflated_with_multiplication_or_literals() {
    let ts = lex(r#"yield* other(); yield * other(); 2 * 3; "yield*"; // yield*
    /* yield* */ yield*other();"#).unwrap();
    let kinds: Vec<_> = ts.iter().map(|t| &t.tok).collect();
    assert_eq!(kinds.iter().filter(|t| matches!(t, Tok::YieldStar)).count(), 2);
    assert_eq!(kinds.iter().filter(|t| matches!(t, Tok::Star)).count(), 2);
    assert!(kinds.iter().any(|t| matches!(t, Tok::Str(s) if s == "yield*")));
    // Spaces around the delegation operator are not silently accepted.
    let err = Parser::new(lex("fnc *gen a(): Generator<int> { yield *other(); }").unwrap())
        .parse_program().unwrap_err();
    assert!(matches!(err.code, "E_SYNTAX" | "E_UNSUPPORTED"));
}

#[test]
fn forbidden_generator_uses_have_precise_diagnostics() {
    for src in [
        "pub fnc main(): int { yield 1; return 0; }",
        "pub fnc main(): int { yield* count_to(2); return 0; }",
    ] {
        let e = Parser::new(lex(src).unwrap()).parse_program().unwrap_err();
        assert_eq!(e.code, "E_YIELD", "{src}");
    }
    for src in [
        "fnc *gen wrong(): Generator<int> { return 5; }",
        "fnc *gen wrong(): Generator<int> { return 5; yield 3; }",
    ] {
        let e = Parser::new(lex(src).unwrap()).parse_program().unwrap_err();
        assert_eq!(e.code, "E_RETURN", "{src}");
    }
    let err = Parser::new(lex("fnc *gen wrong(): int { yield 1; }").unwrap())
        .parse_program().unwrap_err();
    assert_eq!(err.code, "E_TYPE");
}

#[test]
fn malformed_or_unsupported_generator_types_reject_without_a_guest_artifact() {
    for src in [
        "fnc *gen bad(): Generator<string> { yield \"x\"; }",
        "fnc *gen bad(): Generator<int { yield 1; }",
    ] {
        assert!(atto_compiler::compile(src).is_err(), "{src}");
    }
    for src in [
        SAMPLE,
        "fnc *gen foo(): Generator<int> { yield 1; } pub fnc main(): int { return 0; }",
    ] {
        let err = atto_compiler::compile(src).expect_err("no guest artifact until lowering exists");
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}");
        assert!(err.message.contains("continuations"), "{src}");
    }
    let err = atto_compiler::compile("pub fnc main(): Generator<int> { return 0; }").unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED_TYPE");
}
