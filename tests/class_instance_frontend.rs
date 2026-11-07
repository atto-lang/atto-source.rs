//! M2 phase 1: AST and diagnostic contracts. No object Wasm lowering is claimed.
use atto_compiler::{ast::{DeclKind,ExprKind,Type},compile,lexer::lex,parser::Parser};
fn parse(src:&str)->atto_compiler::ast::Program {
    Parser::new(lex(src).expect("lexer")).parse_program().expect("parser")
}
fn reject(src:&str, code:&str) {
    let failure=compile(src).expect_err("unsupported instance operations must not emit a guest");
    assert_eq!(failure.code,code,"{failure} for {src}");
}
const ENTRY:&str="pub fnc main(): int { return 42; }\n";
#[test]
fn preserves_typed_field_layout_and_immutability_intent() {
    let s=format!("define class Counter as\n final int count: 0;\n let ready: bool = false;\n final const int limit: 100;\nend\n{ENTRY}");
    let program=parse(&s);
    assert_eq!(program.decls.len(),1);
    let d=&program.decls[0];
    assert_eq!(d.name,"Counter");
    assert_eq!(d.kind,DeclKind::Class);
    assert_eq!(d.fields.len(),3);
    assert_eq!(&d.fields[0].name,"count");
    assert_eq!(d.fields[0].ty,Type::Int);
    assert!(d.fields[0].final_slot);
    assert!(!d.fields[0].require_const);
    assert!(matches!(d.fields[0].initializer.kind,ExprKind::Int(Some(0))));
    assert_eq!(d.fields[1].ty,Type::Bool);
    assert!(!d.fields[1].final_slot);
    assert!(matches!(d.fields[1].initializer.kind,ExprKind::Bool(false)));
    assert_eq!(d.fields[2].ty,Type::Int);
    assert!(d.fields[2].final_slot);
    assert!(d.fields[2].require_const);
    let guest=compile(&s).expect("primitive class layout produces executable Wasm");
    assert!(guest.starts_with(b"\0asm"));
}
#[test]
fn records_constructor_expr_without_constructing_fictional_object() {
    let src=format!("define class Counter as end\npub fnc main(): int {{\n  val c = new Counter(10, 32);\n  return 42;\n}}\n");
    let program=parse(&src);
    let stmt=&program.funcs[0].body[0];
    let e=match stmt {
        atto_compiler::ast::Stmt::Bind{init,..}=>init,
        other=>panic!("wrong AST: {other:?}"),
    };
    assert!(matches!(&e.kind, ExprKind::New(name,args) if name=="Counter" && args.len()==2));
    reject(&src,"E_ARITY");
}
#[test]
fn rejects_incorrect_field_shapes_before_compiler_codegen() {
    let dup=format!("define class C as\n  final int x: 1;\n  let int x: 2;\nend\n{ENTRY}");
    // The second field above uses a noncanonical local-binding shape and
    // must fail closed instead of silently accepting a duplicate.
    assert!(compile(&dup).is_err());
    let dup2=format!("define class C as\n final int x: 1;\n int x: 2;\nend\n{ENTRY}");
    reject(&dup2,"E_DUPLICATE");
    let invalid=format!("define class C as\n final void empty: 1;\nend\n{ENTRY}");
    reject(&invalid,"E_TYPE");
    let missing=format!("define class C as\n final int x;\nend\n{ENTRY}");
    assert!(compile(&missing).is_err());
}
#[test]
fn never_treats_actor_or_unknown_type_as_constructible_class() {
    reject(&format!("define actor Worker as end\npub fnc main(): int {{ val w = new Worker(); return 0; }}"),
        "E_UNKNOWN_TYPE");
    reject("pub fnc main(): int { val x = new Ghost(); return 0; }","E_UNKNOWN_TYPE");
    reject(&format!("define class C as end\nfnc take(c: C): int {{return 42;}}\n{ENTRY}"),"E_UNSUPPORTED");
    reject(&format!("define class C as end\nfnc clone(c: C): C {{return c;}}\n{ENTRY}"),"E_UNSUPPORTED");
}
#[test]
fn constructor_argument_bound_enforced() {
    let args=(0..33).map(|_|"1").collect::<Vec<_>>().join(",");
    let source=format!("define class C as end\npub fnc main(): int {{ val c = new C({args}); return 0; }}");
    reject(&source,"E_SOURCE_LIMIT");
}
#[test]
fn existing_static_class_and_actor_paths_remain_executable() {
    for kind in ["class","actor"] {
        let src=format!("define {kind} Tool as\n static fnc answer(): int {{return 42;}}\nend\npub fnc main(): int {{return Tool.answer();}}");
        let bytes=compile(&src).expect("static member compile preserved");
        assert!(bytes.starts_with(b"\0asm"));
    }
}
