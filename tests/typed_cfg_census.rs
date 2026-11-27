//! Census from actual parser/checker output, not authored lifetime facts.
use atto_compiler::{check, lexer, parser};
use check::typed_cfg_census::{from_checked_program, PointKind};

fn checked(source: &str) -> check::TProgram {
    let ast = parser::Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    check::check(&ast).unwrap()
}

#[test]
fn actual_typed_branch_has_two_arms_and_shared_return() {
    let program = checked("pub fnc main(): int { let x = 40; if x eq 40 then x = x + 2; else x = 1; fi return x; }");
    let census = from_checked_program(&program).unwrap();
    let main = census.iter().find(|f| f.name == "main").unwrap();
    assert_eq!(main.points[main.entry].kind, PointKind::Set(0));
    let branches: Vec<_> = main.points.iter().filter(|p| p.kind == PointKind::Branch).collect();
    assert_eq!(branches.len(), 1);
    let branch = branches[0];
    assert_eq!(branch.reads, vec![0]);
    let yes = &main.points[branch.successors[0]];
    let no = &main.points[branch.successors[1]];
    assert_eq!(yes.kind, PointKind::Set(0));
    assert_eq!(no.kind, PointKind::Set(0));
    assert_eq!(yes.reads, vec![0]);
    assert!(no.reads.is_empty());
    assert_eq!(yes.successors, no.successors);
    let ret = &main.points[yes.successors[0]];
    assert_eq!(ret.kind, PointKind::Return);
    assert_eq!(ret.reads, vec![0]);
    assert!(ret.successors.is_empty());
    assert_eq!(census, from_checked_program(&program).unwrap());
}

#[test]
fn call_loop_and_heap_effects_do_not_get_partial_certificates() {
    for source in [
        "fnc f(): int { return 42; } pub fnc main(): int { return f(); }",
        "pub fnc main(): int { let x = 0; while x < 42 { x = x + 1; } return x; }",
        "define class C as pub let int n = 42; end pub fnc main(): int { let c = new C(); return c.n; }",
    ] {
        let program = checked(source);
        assert_eq!(from_checked_program(&program).unwrap_err().code, "E_TYPED_CENSUS");
    }
}
