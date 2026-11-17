//! Real AttoLang compiler-owned typed MIR census, not fabricated loan facts.
//! Host-only Rust checks are separate from executable Wasm guest conformance.
use atto_compiler::check::typed_mir_census::{
    checked_borrow_issues, from_checked_program, Operation, CONTRACT,
};
use atto_compiler::check::{TExpr, TFunc, TProgram, TStmt, TE};
use atto_compiler::{check, compile, compile_with_typed_scalar_census, lexer, parser};
use atto_compiler::ast::Type;

fn lowered(source: &str) -> TProgram {
    let toks = lexer::lex(source).unwrap();
    let parsed = parser::Parser::new(toks).parse_program().unwrap();
    check::check(&parsed).unwrap()
}
fn pure() -> &'static str {
    "pub fnc main(): int { let number = 40; val increment = 2; number = number + increment; return number; }"
}
#[test]
fn compiler_derived_origins_match_executable_guest_bytes() {
    let (bytes, census) = compile_with_typed_scalar_census(pure()).unwrap();
    let baseline = compile(pure()).unwrap();
    assert_eq!(bytes, baseline, "census cannot alter guest code generation");
    assert_eq!(census.contract, CONTRACT);
    let main = census.functions.iter().find(|f| f.name == "main").unwrap();
    assert_eq!(main.points.len(), 4);
    assert_eq!(main.points[0].instruction_id, 1);
    assert_eq!(main.points[0].successor, Some(2));
    assert_eq!(main.points[1].successor, Some(3));
    assert!(matches!(main.points[0].operation, Operation::Define { .. }));
    assert!(matches!(main.points[3].operation, Operation::Return));
    // New SSA origin per definition, including replacement of number.
    assert_eq!(main.points[2].used_origins, vec![1, 2]);
    assert_eq!(main.points[3].used_origins, vec![3]);
    assert!(main.points[3].successor.is_none());
}
#[test]
fn typed_phi_and_branch_is_not_guessed_from_a_ternary() {
    let src = "pub fnc main(): int { val a = 40; val b = 2; val result = true ? a : b; return result; }";
    let (bytes, census) = compile_with_typed_scalar_census(src).unwrap();
    assert_eq!(bytes, compile(src).unwrap());
    let main = census.functions.iter().find(|f| f.name == "main").unwrap();
    assert_eq!(main.points[2].used_origins, vec![1, 2], "pure ternary has MAY reads on both arms");
}
#[test]
fn checked_source_borrow_issue_survives_typed_lowering() {
    let src = include_str!("fixtures/pass/ownership-borrow-nll-last-use.atto");
    let program = lowered(src);
    let issues = checked_borrow_issues(&program).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].alias, "reader");
    assert!(!issues[0].exclusive);
    // The checked compiler emits successful native guest Wasm normally.
    let guest = compile(src).unwrap();
    assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    // But no complete borrow use/end facts exist in typed IR yet.
    let error = compile_with_typed_scalar_census(src).unwrap_err();
    assert_eq!(error.code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
    assert!(error.message.contains("loan-use/end census"));
}
#[test]
fn real_exclusive_borrow_issue_preserves_owner_and_kind() {
    let src = include_str!("fixtures/pass/ownership-borrow-nll-exclusive-transition.atto");
    let issues = checked_borrow_issues(&lowered(src)).unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].alias, "reader");
    assert!(!issues[0].exclusive);
    assert_eq!(issues[1].alias, "writer");
    assert!(issues[1].exclusive);
    assert_eq!(issues[0].owner_slot, issues[1].owner_slot);
}
#[test]
fn compiler_rejects_unsafe_guest_before_a_census_can_authorize_it() {
    let bad = include_str!("fixtures/fail/ownership-borrow-nll-live-after-mutation.atto");
    assert_eq!(compile(bad).unwrap_err().code, "E_BORROW");
    assert_eq!(compile_with_typed_scalar_census(bad).unwrap_err().code, "E_BORROW");
}
#[test]
fn calls_and_control_flow_fail_closed_in_opt_in_census_not_default_compilation() {
    let call = "fnc plus(int a, int b): int { return a + b; } pub fnc main(): int { return plus(40, 2); }";
    assert!(compile(call).is_ok());
    assert_eq!(compile_with_typed_scalar_census(call).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
    let branch = "pub fnc main(): int { if true then return 42; else return 4; fi }";
    assert!(compile(branch).is_ok());
    assert_eq!(compile_with_typed_scalar_census(branch).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
}
fn fabricated(expr: TExpr) -> TProgram {
    TProgram {
        funcs: vec![TFunc {
            name: "main".to_string(),
            params: vec![],
            ret: Type::Int,
            locals: vec![Type::Int],
            body: vec![TStmt::Return(Some(TE { e: expr, ty: Type::Int }))],
        }],
        generators: vec![],
        main: 0,
        imports: vec![],
        data: vec![],
        heap: false,
        globals: vec![],
        actors: vec![],
        strings: false,
        guest_eh: false,
    }
}
#[test]
fn malformed_typed_local_reads_are_never_invented() {
    for slot in [0, 42] {
        let p = fabricated(TExpr::Local(slot));
        assert_eq!(from_checked_program(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_INCOMPLETE");
    }
}
#[test]
fn effectful_expression_cannot_be_pretended_pure() {
    let p = fabricated(TExpr::Call(0, vec![]));
    assert_eq!(from_checked_program(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_UNSUPPORTED");
}
#[test]
fn deep_typed_expression_exhausts_bound_without_unbounded_recursion() {
    let mut expr = TE { e: TExpr::I64(42), ty: Type::Int };
    for _ in 0..150 {
        expr = TE { e: TExpr::Neg(Box::new(expr)), ty: Type::Int };
    }
    let p = fabricated(expr.e);
    assert_eq!(from_checked_program(&p).unwrap_err().code, "E_TYPED_MIR_CENSUS_BUDGET");
}
#[test]
fn source_and_typed_census_are_deterministic() {
    let (_, a) = compile_with_typed_scalar_census(pure()).unwrap();
    let (_, b) = compile_with_typed_scalar_census(pure()).unwrap();
    assert_eq!(a, b);
}
