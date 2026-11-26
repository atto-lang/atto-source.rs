use atto_compiler::compile_verified_strict;
const PRE:&str="type Bounded = int where self >= 1 and self <= 42; ";
fn source(open:&str,close:&str,clauses:&str,args:&str)->String{
    format!("{PRE}fnc divide(int a, int b) -> int {clauses}{open} return a / b; {close} pub fnc main() -> int {{ return divide({args}); }}")
}
#[test]
fn bounded_contract_styles_are_identical(){
    let baseline=compile_verified_strict(&source(" {","}","requires b != 0 requires b != -1","84, 2")).unwrap();
    for (open,close) in [(" {","}"),("\nbegin","end"),(" ::","end"),("\n::","end")]{
        let safe=source(open,close,"requires b != 0 requires b != -1","84, 2");
        assert_eq!(compile_verified_strict(&safe).unwrap(),baseline);
        for (clauses,args) in [("requires b != 0","84, 2"),("requires b != 0 requires b != -1","84, 0"),("requires b != 0 requires b == 0","84, 2")]{
            assert!(compile_verified_strict(&source(open,close,clauses,args)).is_err());
        }
    }
}
#[test]
fn nested_braces_do_not_close_end_delimited_callable(){
    let body="fnc f(int x) -> int requires x >= 1 requires x <= 42";
    let tail=" if x == 2 { return 84/x; } return 42; ";
    let main=" pub fnc main() -> int { return f(2); }";
    let baseline=compile_verified_strict(&format!("{PRE}{body} {{{tail}}}{main}")).unwrap();
    assert_eq!(compile_verified_strict(&format!("{PRE}{body} ::{tail}end{main}")).unwrap(),baseline);
    let missing=source(" ::","","requires b != 0 requires b != -1","84, 2");
    assert_eq!(compile_verified_strict(&missing).unwrap_err().code,"E_SYNTAX");
    for (open,close) in [(" begin","end"),(" ::","}"),(" ::",""),(" :: begin","end")]{
        assert!(compile_verified_strict(&source(open,close,"requires b != 0 requires b != -1","84, 2")).is_err());
    }
}
