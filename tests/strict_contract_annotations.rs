use atto_compiler::compile_verified_strict;
const PRE:&str="type Bounded = int where self >= 1 and self <= 42; ";
fn source(annotations:&str,clauses:&str,open:&str,close:&str,args:&str)->String {
    format!("{PRE}{annotations}fnc divide(int a, int b) -> int {clauses}{open} return a / b; {close} pub fnc main() -> int {{ return divide({args}); }}")
}
#[test]
fn annotations_and_keyword_contracts_share_typed_proofs() {
    for (open,close) in [(" {","}"),("\nbegin","end"),(" ::","end"),("\n::","end")]{
        let baseline=compile_verified_strict(&source("","requires b != 0 requires b != -1",open,close,"84, 2")).unwrap();
        for (a,k) in [("@requires(b != 0)\n@requires(b != -1)\n",""),("@requires(b != 0)\n","requires b != -1"),("@requires(b != -1)\n","requires b != 0")]{
            assert_eq!(compile_verified_strict(&source(a,k,open,close,"84, 2")).unwrap(),baseline);
            assert!(compile_verified_strict(&source(a,k,open,close,"84, 0")).is_err());
            assert!(compile_verified_strict(&source(a,k,open,close,"84, -1")).is_err());
        }
    }
}
#[test]
fn unsupported_or_contradictory_annotations_fail_closed() {
    for (a,k) in [("@requires(b != 0)\n","requires b == 0"),("@requires(unknown != 0)\n","requires b != -1"),("@requires(b != 0 && b != -1)\n",""),("@requires(b != 0\n",""),("@foo(b != 0)\n",""),("@requires(b != 0)\n@ensures(result == a / b)\n","requires b != -1")]{
        assert!(compile_verified_strict(&source(a,k," ::","end","84, 2")).is_err());
    }
    assert!(compile_verified_strict(&source("@requires(b != 0)\n","requires b != -1"," begin","end","84, 2")).is_err());
    let too_many="@requires(b != 0)\n".repeat(17);
    assert!(compile_verified_strict(&source(&too_many,"requires b != -1"," ::","end","84, 2")).is_err());
}
