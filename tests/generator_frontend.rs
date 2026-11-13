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
    // Delegation still requires typed child continuation ownership and must
    // fail closed. Pure, finite int generators now have real guest Wasm.
    let err = atto_compiler::compile(SAMPLE).expect_err("yield* is not executable yet");
    assert_eq!(err.code, "E_UNSUPPORTED");
    let executable = atto_compiler::compile(
        "fnc *gen foo(): Generator<int> { yield 1; } pub fnc main(): int { return 0; }"
    ).expect("finite int generator must compile to actual Wasm");
    assert!(executable.starts_with(b"\0asm\x01\0\0\0"));
    let err = atto_compiler::compile("pub fnc main(): Generator<int> { return 0; }").unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED_TYPE");
}

#[test]
fn plus_assign_desugars_to_checked_binary_addition_without_disabling_mutability_checks() {
    use atto_compiler::ast::BinOp;
    let p = parse("pub fnc main(): int { let sum = 0; sum += 41; return sum + 1; }");
    assert!(matches!(&p.funcs[0].body[1], Stmt::Assign { name, value, .. }
        if name == "sum" && matches!(&value.kind, ExprKind::Binary(BinOp::Add, _, _))));
    let guest = atto_compiler::compile("pub fnc main(): int { let sum = 0; sum += 41; return sum + 1; }")
        .expect("plain compound addition must compile to real Wasm");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(
        atto_compiler::compile("pub fnc main(): int { val sum = 0; sum += 1; return sum; }")
            .unwrap_err().code, "E_IMMUTABLE"
    );
}

#[test]
fn generator_item_and_delegation_type_checks_precede_wasm_unsupported_gate() {
    let cases = [
        // Literal yield item has the wrong element type.
        "fnc *gen bad(): Generator<int> { yield true; }",
        "fnc *gen bad(): Generator<int> { yield \"wrong\"; }",
        // Delegation must not consume a scalar directly.
        "fnc *gen bad(): Generator<int> { yield* 7; }",
        "fnc *gen bad(): Generator<int> { yield* false; }",
        // A known non-generator call cannot be delegated to.
        "fnc scalar(): int { return 2; }\nfnc *gen bad(): Generator<int> { yield* scalar(); }",
        // Nested branches must not evade validation.
        "fnc *gen bad(): Generator<int> { if true then yield* 9; fi }",
        "fnc *gen bad(): Generator<int> { for (let i = 0; i < 1; i++) do yield false; done }",
    ];
    for source in cases {
        let err = atto_compiler::compile(source).expect_err(source);
        assert_eq!(err.code, "E_TYPE", "{source}: {err}");
    }
    let valid_delegation = r#"
        fnc *gen inner(): Generator<int> { yield 1; }
        fnc *gen outer(): Generator<int> { yield* inner(); }
        pub fnc main(): int { return 0; }
    "#;
    let bytes = atto_compiler::compile(valid_delegation)
        .expect("zero-argument finite yield* delegates compile to real guest Wasm");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
}
