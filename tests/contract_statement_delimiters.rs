//! Candidate bounded contract grammar and verifier regressions.
use atto_compiler::{compile,compile_verified_strict};

fn program(clauses:&str, opener:&str, closer:&str, args:&str)->String {
    format!("fnc divide(int a, int b) -> int\n{clauses}{opener}\n    return a / b;\n{closer}\npub fnc main() -> int {{ return divide({args}); }}")
}
#[test]
fn three_delimiters_compile_identically() {
    let cases=[(" {","}"),("\n{","}"),("\nbegin","end"),(" ::","end"),("\n::","end")];
    let baseline=compile(&program("    requires b != 0"," {","}","84, 2")).expect("brace");
    for (open,close) in cases {
        let source=program("    requires b != 0",open,close,"84, 2");
        assert_eq!(compile(&source).unwrap_or_else(|e|panic!("{source}: {e}")),baseline);
        let safe=program("    requires b != 0\n    requires b != -1",open,close,"84, 2");
        assert!(compile_verified_strict(&safe).is_ok(),"{safe}");
        assert!(compile_verified_strict(&source).is_err(),"nonzero alone does not exclude MIN/-1");
    }
}
#[test]
fn invalid_contracts_and_delimiters_never_emit() {
    for (clause,open,close,args) in [
        ("    requires b != 0"," begin","end","84, 2"),
        ("    requires b != 0"," ::","}","84, 2"),
        ("    requires b != 0"," ::","","84, 2"),
        ("    requires b != 0"," :: begin","end","84, 2"),
        ("    requires b != 0"," ::","end","84, 0"),
        ("    requires b != 0\n    requires b == 0"," ::","end","84, 2"),
        ("    requires b != 2"," ::","end","84, 3"),
    ] {
        let source=program(clause,open,close,args);
        assert!(compile(&source).is_err(),"{source}");
    }
}
#[test]
fn checked_cast_in_body_is_not_a_delimiter() {
    let source="fnc convert(int a) -> int\n    requires a >= 0 ::\n    return a::<i64>;\nend\npub fnc main() -> int { return convert(42); }";
    assert!(compile(source).is_ok());
}
