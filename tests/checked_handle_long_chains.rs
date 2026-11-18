//! The checked-handle security gate must not treat a long arithmetic
//! expression's left-associative AST spine as 128 lexical nesting levels.
//! It must still reject every checked-handle prefix before guest emission.
//! These tests cover both ordinary parsed sources and hostile manual ASTs.
use atto_compiler::{
    ast::{BinOp, Expr, ExprKind, Func, Program, Stmt, Type},
    check,
    diag::Pos,
};

fn addition_chain(n: usize) -> String {
    std::iter::repeat("1").take(n).collect::<Vec<_>>().join(" + ")
}

#[test]
fn ordinary_250_operator_chain_compiles_without_security_gate_nesting_error() {
    // The ordinary source compiler and the browser compiler share this gate.
    // Keep this as real emitted Wasm, not only an AST walker test.
    let src = format!("pub fnc main(): int {{ return {}; }}", addition_chain(250));
    let wasm = atto_compiler::compile(&src).expect("250-term chain must compile");
    assert!(wasm.starts_with(&[0, 97, 115, 109]));
    assert_eq!(wasm, atto_compiler::compile(&src).expect("deterministic compile"));
}

#[test]
fn checked_handles_in_large_chains_reject_before_any_guest_bytes() {
    for form in ["ref! 1", "deref! 1", "unref! 1"] {
        // The checked handle occurs deep in the left spine, which must not
        // be skipped by the non-recursive admission path.
        let mut values = vec!["1".to_owned(); 250];
        values[20] = form.to_owned();
        let src = format!("pub fnc main(): int {{ return {}; }}", values.join(" + "));
        let err = atto_compiler::compile(&src).expect_err("unverified handle cannot lower");
        assert_eq!(err.code, "E_UNSUPPORTED_HANDLE", "{form}: {err}");
    }
}

#[test]
fn binary_chain_budget_still_rejects_untrusted_manual_ast() {
    // Parsed source normally has its own bounded parser budget; hostile
    // embedding APIs can call check::check on an AST directly. Admission
    // must remain bounded even if the parser did not create the tree.
    let pos = Pos { line: 2, col: 5 };
    let mut expr = Expr { pos, kind: ExprKind::Int(Some(1)) };
    for _ in 0..1_100 {
        expr = Expr {
            pos,
            kind: ExprKind::Binary(
                BinOp::Add,
                Box::new(expr),
                Box::new(Expr { pos, kind: ExprKind::Int(Some(1)) }),
            ),
        };
    }
    let mut program = Program::default();
    program.funcs.push(Func {
        name: "main".into(),
        pos,
        is_generator: false,
        is_pure: false,
        params: Vec::new(),
        ret: Type::Int,
        body: vec![Stmt::Return(Some(expr), pos)],
    });
    let err = check::check(&program).expect_err("hostile oversized spine must reject");
    assert_eq!(err.code, "E_NESTING");
    assert_eq!((err.pos.line, err.pos.col), (2, 5));
}

#[test]
fn deeply_nested_nonbinary_expressions_keep_original_depth_limit() {
    use atto_compiler::ast::UnOp;
    // Parser versions may reject repeated unary +/- as an unsupported
    // source spelling before the security gate. Construct a deliberately
    // hostile AST instead to prove that the checked-handle preflight
    // *itself* still rejects genuine nesting at MAX_DEPTH.
    let pos = Pos { line: 7, col: 4 };
    let mut value = Expr { pos, kind: ExprKind::Int(Some(42)) };
    for _ in 0..150 {
        value = Expr {
            pos,
            kind: ExprKind::Unary(UnOp::Plus, Box::new(value)),
        };
    }
    let mut program = Program::default();
    program.funcs.push(Func {
        name: "main".into(), pos, is_generator: false, is_pure: false,
        params: Vec::new(), ret: Type::Int,
        body: vec![Stmt::Return(Some(value), pos)],
    });
    let err = check::check(&program).expect_err("real unary AST nesting rejects");
    assert_eq!(err.code, "E_NESTING");
    assert_eq!((err.pos.line, err.pos.col), (7, 4));
}
