//! Compiler-derived portable SSA/CFG evidence format is separate from
//! executable guest bytes. It never approves a production borrow rejection.
use atto_compiler::{
    check::{
        self,
        loan_cfg_wire_v1::{self, CONTRACT},
    },
    compile, lexer,
    parser::Parser,
};

fn manifest(source: &str) -> Result<String, atto_compiler::Diagnostic> {
    let ast = Parser::new(lexer::lex(source)?).parse_program()?;
    let checked = check::check(&ast)?;
    loan_cfg_wire_v1::export_checked_program(&checked)
}
fn straight() -> &'static str {
    "pub fnc main(): int { let number = 40; val reader = borrow! number; val result = copy! reader; number = 42; return number; }"
}
fn two_branches() -> &'static str {
    "pub fnc main(): int {
      let number = 40;
      val choose = true;
      if choose then
          val reader = borrow! number;
          val snapshot = copy! reader;
      else
          val alternative = borrow! number;
          val duplicate = copy! alternative;
      fi
      number = 42;
      return number;
    }"
}
#[test]
fn real_compiled_scalar_borrow_exports_all_checked_loan_edges() {
    let source = straight();
    assert_eq!(&compile(source).unwrap()[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    let wire = manifest(source).unwrap();
    assert!(wire.starts_with(&format!("{CONTRACT}\n")));
    assert!(wire.ends_with("END\n"));
    let rows = wire.lines().filter(|l| l.starts_with("P|")).count();
    let declared: usize = wire
        .lines()
        .find(|l| l.starts_with("F|"))
        .unwrap()
        .split('|')
        .nth(3)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(rows, declared);
    assert!(wire.contains("|I|1|0|0|0|"));
    assert!(wire.contains("|M|1|0|0|0|"));
    assert!(wire.contains("|E|1|0|0|0|"));
    assert!(wire.contains("|1:0|"));
    assert_eq!(manifest(source).unwrap(), wire);
}
#[test]
fn actual_branch_scopes_are_serialized_with_all_cfg_edges() {
    let source = two_branches();
    assert!(compile(source).is_ok());
    let wire = manifest(source).unwrap();
    assert!(wire.matches("|I|").count() >= 2);
    assert!(wire.matches("|E|").count() >= 2);
    let header: Vec<_> = wire.lines().find(|l| l.starts_with("F|")).unwrap().split('|').collect();
    let count: usize = header[3].parse().unwrap();
    assert_eq!(count, wire.lines().filter(|l| l.starts_with("P|")).count());
    assert!(wire.lines().any(|l| l.contains("|C|")));
}
#[test]
fn unsupported_compiler_cfg_is_never_exported_as_valid_evidence() {
    let bad = include_str!("fixtures/fail/ownership-borrow-nll-live-after-mutation.atto");
    assert!(compile(bad).is_err());
    assert!(manifest(bad).is_err());
    let with_call = include_str!("fixtures/pass/ownership-borrow-nll-last-use.atto");
    assert!(compile(with_call).is_ok());
    assert!(manifest(with_call).is_err(), "effectful call requires exceptional CFG");
}
#[test]
fn different_checked_programs_have_distinct_complete_wire_inventories() {
    let a = manifest(straight()).unwrap();
    let b = manifest(two_branches()).unwrap();
    assert_ne!(a, b);
    assert_eq!(a.lines().filter(|l| l == "END").count(), 1);
    assert_eq!(b.lines().filter(|l| l == "END").count(), 1);
}
