//! Production parser and type-checker boundary for prefix `<< readable`.
//! These are AST/checker tests, not executable guest-channel ABI evidence.
use atto_compiler::{ast::{Expr, ExprKind, Stmt}, check, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("lex")).parse_program().expect("parse")
}

fn count_reads(e: &Expr) -> usize {
    match &e.kind {
        ExprKind::PrefixRead(source) => 1 + count_reads(source),
        ExprKind::Binary(_, a, b) => count_reads(a) + count_reads(b),
        ExprKind::Unary(_, inner) | ExprKind::Await(inner)
        | ExprKind::Trap(inner) | ExprKind::Propagate(inner) => count_reads(inner),
        ExprKind::Call(_, args) => args.iter().map(count_reads).sum(),
        _ => 0,
    }
}

#[test]
fn prefix_read_has_its_own_ast_node_in_value_positions() {
    let program = parse(
        "pub suspends fnc main(): int { \
         val a = << c; \
         val b = (<< c); \
         process(<< c); \
         return (<< c) + (<< c); }"
    );
    let stmts = &program.funcs[0].body;
    assert!(matches!(&stmts[0], Stmt::Bind { init: Expr {kind: ExprKind::PrefixRead(_), ..}, .. }));
    assert!(matches!(&stmts[1], Stmt::Bind { init: Expr {kind: ExprKind::PrefixRead(_), ..}, .. }));
    assert!(matches!(&stmts[2], Stmt::Expr(Expr { kind: ExprKind::Call(_, _), .. })));
    let Stmt::Return(Some(sum), _) = &stmts[3] else { panic!("expected return") };
    assert_eq!(count_reads(sum), 2, "two occurrences are two ordered read nodes");
    assert_eq!(count_reads(match &stmts[2] {
        Stmt::Expr(e) => e,
        _ => unreachable!(),
    }), 1);
}

#[test]
fn compound_assignment_preserves_exactly_one_read_node() {
    for rhs in ["<< c", "(<< c)"] {
        let src = format!(
            "pub susp fnc main(): int {{ let total = 0; total += {rhs}; return total; }}"
        );
        let program = parse(&src);
        let Stmt::Assign { value, .. } = &program.funcs[0].body[1] else {
            panic!("expected local compound assignment lowering");
        };
        assert_eq!(count_reads(value), 1, "{rhs} must read once");
        assert!(matches!(value.kind, ExprKind::Binary(_, _, _)));
    }
}

#[test]
fn direct_checker_rejects_nonreadable_operand_and_missing_suspension() {
    let code = "pub suspends fnc main(): int { val n = 42; val x = << n; return 0; }";
    let p = parse(code);
    let err = check::check(&p).expect_err("scalar cannot act as readable");
    assert_eq!(err.code, "E_IO_READABLE", "{err}");

    let p = parse("pub fnc main(): int { val n = 42; val x = << n; return 0; }");
    let err = check::check(&p).expect_err("sync function cannot read");
    assert_eq!(err.code, "E_SUSPENDS_REQUIRED", "{err}");
}

#[test]
fn unbound_prefix_read_cannot_emerge_as_guest_wasm() {
    let src = "pub susp fnc main(): int { val x = << c; return 0; }";
    let err = atto_compiler::compile(src).expect_err("unbound source channel must fail before guest emission");
    assert_eq!(err.code, "E_UNBOUND", "{err}");
}
