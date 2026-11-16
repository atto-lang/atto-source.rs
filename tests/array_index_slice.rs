//! Executable Array<int> candidate: compile real AttoLang, then the existing
//! Node conformance harness executes each guest Wasm pass/panic fixture.
//! These native tests by themselves prove compilation, not guest execution.
use atto_compiler::{ast::{ExprKind, Stmt}, lexer, parser::Parser};

#[test]
fn index_and_slice_are_distinct_source_nodes() {
    let src = "pub fnc main(): int { val a = [1,2,3,4]; return a[1:4][-1]; }";
    let p = Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    let Stmt::Return(Some(e), _) = &p.funcs[0].body[1] else {
        panic!("expected return statement");
    };
    let ExprKind::Index(base, index) = &e.kind else {
        panic!("index node missing");
    };
    assert!(matches!(&base.kind, ExprKind::Slice { .. }));
    assert!(matches!(&index.kind, ExprKind::Unary(..)));
}
#[test]
fn codegen_accepts_borrowed_contiguous_array_slices_without_copies() {
    for body in [
        "val a = [2,4,6]; return a[-1];",
        "val a = [2,4,6]; return a[1:3][0];",
        "val a = [2,4,6]; return a[:][2];",
        "val a = [2,4,6]; return a[::1][-1];",
        "val a: Array<int> = []; val s = a[:]; return 0;",
        "val a = [2,4,6]; val s = a[2:1]; return 0;",
    ] {
        let src = format!("pub fnc main(): int {{ {body} }}");
        let wasm = atto_compiler::compile(&src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(wasm.starts_with(b"\\0asm"), "{src}");
        assert_eq!(wasm, atto_compiler::compile(&src).unwrap(), "deterministic codegen");
    }
}
#[test]
fn malformed_and_unimplemented_strides_fail_closed() {
    for (body, want) in [
        ("val a = [1]; return a[];", "E_SLICE_SYNTAX"),
        ("val a = [1]; val s = a[::0]; return 0;", "E_SLICE_STEP_ZERO"),
        ("val a = [1]; val s = a[1:2:]; return 0;", "E_SLICE_SYNTAX"),
        ("val a = [1]; val s = a[::-1]; return 0;", "E_UNSUPPORTED"),
        ("val a = [1]; val s = a[::2]; return 0;", "E_UNSUPPORTED"),
        ("val a = [1]; return a[true];", "E_TYPE"),
        ("return 42[0];", "E_NOT_INDEXABLE"),
        ("val a = 42[:]; return 0;", "E_NOT_SLICEABLE"),
    ] {
        let src = format!("pub fnc main(): int {{ {body} }}");
        let err = atto_compiler::compile(&src).expect_err(&src);
        assert_eq!(err.code, want, "{src}: {err}");
    }
}
