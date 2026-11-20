//! Parser admission only: typed indexed lvalues remain deliberately
//! unlowered until Map/Record mutability, layout and ownership are proven.
use atto_compiler::{ast::{ExprKind, Stmt}, compile, lexer, parser::Parser};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap()
}

#[test]
fn record_index_assignment_preserves_receiver_key_and_rhs_ast() {
    let p = parse(r#"pub fnc main(): int {
        let mut Record record = {foo: 1};
        record["foo"] = 3;
        return record["foo"];
    }"#);
    match &p.funcs[0].body[1] {
        Stmt::IndexAssign {target, index, value, ..} => {
            assert!(matches!(&target.kind, ExprKind::Var(n) if n == "record"));
            assert!(matches!(&index.kind, ExprKind::Str(n) if n == "foo"));
            assert!(matches!(&value.kind, ExprKind::Int(Some(3))));
        }
        _ => panic!("typed indexing must remain a separate checked AST lvalue"),
    }
    assert!(matches!(&p.funcs[0].body[2],
        Stmt::Return(Some(v), _) if matches!(&v.kind, ExprKind::Index(_, _))));
}

#[test]
fn record_nonidentifier_quoted_key_is_not_rewritten_into_member() {
    let p = parse(r#"pub fnc main(): int {
        let mut Record record = {"content-type": 1};
        record["content-type"] = 7;
        return 0;
    }"#);
    assert!(matches!(&p.funcs[0].body[1],
        Stmt::IndexAssign {index, ..}
        if matches!(&index.kind, ExprKind::Str(k) if k == "content-type")));
}

#[test]
fn map_index_assignment_is_not_a_record_field() {
    let p = parse(r#"pub fnc main(): int {
        let mut Map<string,int> map = {"foo" => 1};
        map["foo"] = 3;
        return 0;
    }"#);
    assert!(matches!(&p.funcs[0].body[1], Stmt::IndexAssign { .. }));
    assert!(matches!(&p.funcs[0].body[0],
        Stmt::Bind {init, ..} if matches!(&init.kind, ExprKind::Map(_))));
}

#[test]
fn nominal_struct_is_not_granted_index_write_authority() {
    let src = r#"define struct Point as int foo; end
        pub fnc main(): int {
          let mut p = Point{foo: 1};
          p["foo"] = 3;
          return 0;
        }"#;
    let p = parse(src);
    assert!(matches!(&p.funcs[0].body[1], Stmt::IndexAssign { .. }));
    let error = compile(src).expect_err("unverified Struct lvalue must fail closed");
    assert_eq!(error.code, "E_UNSUPPORTED");
}

#[test]
fn all_unlowered_index_writes_produce_no_guest_wasm() {
    for src in [
        r#"pub fnc main(): int { let mut Record r = {foo:1}; r["foo"] = 3; return r.foo; }"#,
        r#"pub fnc main(): int { let mut Map<string,int> m = {"foo" => 1}; m["foo"] = 3; return 0; }"#,
        r#"pub fnc main(): int { let mut Record r = {foo:1}; let string key="foo"; r[key] = 3; return 0; }"#,
    ] {
        let err = compile(src).expect_err("indexed write needs verified checked guest lowering");
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}: {err}");
    }
}

#[test]
fn invalid_index_assign_syntax_stays_invalid() {
    let src = r#"pub fnc main(): int {
        let mut Record r = {foo:1};
        r["foo" = 3;
        return 0;
    }"#;
    let error = Parser::new(lexer::lex(src).unwrap()).parse_program()
        .expect_err("missing closing bracket");
    assert_eq!(error.code, "E_SYNTAX");
}
