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
fn call_and_heap_effects_do_not_get_partial_certificates() {
    for source in [
        "fnc f(): int { return 42; } pub fnc main(): int { return f(); }",
        "define class C as pub let int n = 42; end pub fnc main(): int { let c = new C(); return c.n; }",
    ] {
        let program = checked(source);
        assert_eq!(from_checked_program(&program).unwrap_err().code, "E_TYPED_CENSUS");
    }
}

#[test]
fn checked_for_continue_routes_through_update_before_backedge() {
    let program = checked(include_str!("fixtures/pass/18-continue-in-for.atto"));
    let graph = from_checked_program(&program).unwrap();
    let main = graph.iter().find(|f| f.name == "main").unwrap();
    let (head_id, head) = main.points.iter().enumerate()
        .find(|(_, p)| p.kind == PointKind::LoopHead).unwrap();
    assert_eq!(head.successors.len(), 2);
    let jump = main.points.iter().find(|p| p.kind == PointKind::Continue).unwrap();
    let update = &main.points[jump.successors[0]];
    assert!(matches!(update.kind, PointKind::Set(_)));
    assert_eq!(update.successors, vec![head_id]);
    assert_eq!(main.points[head.successors[1]].kind, PointKind::Return);
}

#[test]
fn checked_unconditional_loop_break_targets_following_return() {
    let program = checked(include_str!("fixtures/pass/16-loop-break.atto"));
    let graph = from_checked_program(&program).unwrap();
    let main = graph.iter().find(|f| f.name == "main").unwrap();
    let head = main.points.iter().find(|p| p.kind == PointKind::LoopHead).unwrap();
    assert_eq!(head.successors.len(), 1);
    let jump = main.points.iter().find(|p| p.kind == PointKind::Break).unwrap();
    assert_eq!(main.points[jump.successors[0]].kind, PointKind::Return);
    assert!(main.points.iter().flat_map(|p| &p.successors).all(|&id| id < main.points.len()));
}

#[test]
fn nested_loop_transfers_bind_to_nearest_loop() {
    let program = checked("pub fnc main(): int { let x = 0; loop { loop { x = x + 1; break; } if x eq 42 then break; fi continue; } return x; }");
    let graph = from_checked_program(&program).unwrap();
    let main = graph.iter().find(|f| f.name == "main").unwrap();
    let breaks: Vec<_> = main.points.iter().filter(|p| p.kind == PointKind::Break).collect();
    assert_eq!(breaks.len(), 2);
    assert_ne!(breaks[0].successors, breaks[1].successors);
    assert_eq!(breaks.iter().filter(|p| main.points[p.successors[0]].kind == PointKind::Return).count(), 1);
    let cont = main.points.iter().find(|p| p.kind == PointKind::Continue).unwrap();
    assert_eq!(main.points[cont.successors[0]].kind, PointKind::LoopHead);
}
