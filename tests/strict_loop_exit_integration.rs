use atto_compiler::compile_verified_strict;
const PRE:&str="type Count = int where self >= 0 and self <= 42; ";
#[test]
fn loop_exits_are_scoped_and_do_not_inherit_false_guard() {
 for body in ["let int x=0; while x<42 { if x==0 { break; } x=x+1; } return 42/x;",
 "let Count x=0; while x<42 { while x<42 { break; } if x==0 { break; } x=x+1; } return 42/x;"] {
 let s=format!("{PRE} pub fnc main() -> int {{ {body} }}");
 assert!(compile_verified_strict(&s).unwrap_err().render().contains("E_VERIFY_UNPROVEN"),"{body}");
 }
 for body in ["break; return 42;","continue; return 42;"] {
 let s=format!("{PRE} pub fnc main() -> int {{ {body} }}");
 assert!(compile_verified_strict(&s).unwrap_err().render().contains("E_VERIFY_UNSUPPORTED"));
 }
}
