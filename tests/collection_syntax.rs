//! Frontend syntax tests only: executable guest collection support is NOT claimed.
use atto_compiler::{
    ast::{ExprKind, Stmt},
    lexer::{self, Tok},
    parser::Parser,
};

fn parse_body(body: &str) -> Result<Vec<Stmt>, atto_compiler::Diagnostic> {
    let source = format!("pub fnc main(): int {{ {body} return 0; }}");
    let mut p = Parser::new(lexer::lex(&source)?).parse_program()?;
    Ok(p.funcs.remove(0).body)
}
fn init<'a>(statements: &'a [Stmt], name: &str) -> &'a ExprKind {
    for statement in statements {
        if let Stmt::Bind { name: bound, init, .. } = statement {
            if bound == name { return &init.kind; }
        }
    }
    panic!("missing local {name}");
}
#[test]
fn spread_is_atomic_and_member_dot_and_floats_still_lex() {
    let tokens = lexer::lex("[...items, value.field, 1.5]").unwrap();
    assert!(tokens.iter().any(|t| t.tok == Tok::Spread));
    assert!(tokens.iter().any(|t| t.tok == Tok::Dot));
    assert!(tokens.iter().any(|t| matches!(t.tok, Tok::Float(_))));
}
#[test]
fn index_and_all_slice_forms_have_distinct_ast_nodes() {
    let stmts = parse_body("
      val a = [10,20,30,40,50];
      val first = a[0];
      val last = a[-1];
      val mid = a[1:4];
      val head = a[:3];
      val tail = a[2:];
      val all = a[:];
      val stride = a[::2];
      val backwards = a[::-1];
      val rev_part = a[4:0:-2];
      val spaced = a[: : 2];
      val chained = a[1:4][0];
    ").unwrap();
    assert!(matches!(init(&stmts, "first"), ExprKind::Index(..)));
    assert!(matches!(init(&stmts, "last"), ExprKind::Index(..)));
    for (name, start, end, step) in [
        ("mid",true,true,false),("head",false,true,false),
        ("tail",true,false,false),("all",false,false,false),
        ("stride",false,false,true),("backwards",false,false,true),
        ("rev_part",true,true,true),("spaced",false,false,true)
    ] {
        let ExprKind::Slice { start: s, end: e, step: k, .. } = init(&stmts, name) else {
            panic!("{name} must parse as a slice");
        };
        assert_eq!((s.is_some(), e.is_some(), k.is_some()), (start,end,step), "{name}");
    }
    assert!(matches!(init(&stmts, "chained"), ExprKind::Index(base, _) if
        matches!(base.kind, ExprKind::Slice { .. })));
}
#[test]
fn spread_only_parses_in_collection_and_call_contexts() {
    let stmts = parse_body("
        val a = [1,2];
        val joined = [0, ...a, 3];
        val projected = [...a.map(|n| -> n * 2)];
        val called = collect(...a);
    ").unwrap();
    let ExprKind::Array(items) = init(&stmts, "joined") else { panic!("array"); };
    assert_eq!(items.len(), 3);
    assert!(matches!(items[1].kind, ExprKind::Spread(..)));
    let ExprKind::Array(items) = init(&stmts, "projected") else { panic!("array"); };
    assert!(matches!(items[0].kind, ExprKind::Spread(..)));
    assert!(matches!(init(&stmts, "called"), ExprKind::Call(_, args) if
        matches!(args[0].kind, ExprKind::Spread(..))));
}
#[test]
fn Unicode_scalar_slice_is_recognized_without_claiming_execution() {
    let stmts = parse_body("val text = \"héllo\"; val a = text[1]; val b = text[:2];").unwrap();
    assert!(matches!(init(&stmts, "a"), ExprKind::Index(..)));
    assert!(matches!(init(&stmts, "b"), ExprKind::Slice { .. }));
}
#[test]
fn malformed_slice_forms_fail_with_specific_diagnostics() {
    for code in [
        "val a = [1,2]; val x = a[];",
        "val a = [1,2]; val x = a[::];",
        "val a = [1,2]; val x = a[1:2:];",
        "val a = [1,2]; val x = a[1::];",
        "val a = [1,2]; val x = a[0:1:2:3];",
        "val a = [1,2]; val x = a[:::2];",
    ] {
        let err = parse_body(code).unwrap_err();
        assert_eq!(err.code, "E_SLICE_SYNTAX", "{code}: {err}");
    }
    for code in [
        "val a = [1,2]; val x = a[::0];",
        "val a = [1,2]; val x = a[0:2:-0];",
    ] {
        let err = parse_body(code).unwrap_err();
        assert_eq!(err.code, "E_SLICE_STEP_ZERO", "{code}: {err}");
    }
}
#[test]
fn valid_parse_never_claims_guest_execution() {
    let source = "pub fnc main(): int { val a = [1,2,3]; val x = a[::2]; return 0; }";
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
    let source = "pub fnc main(): int { val a = [1,2,3]; val x = [...a]; return 0; }";
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}
