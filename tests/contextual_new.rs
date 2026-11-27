//! Typed-local contextual new must use the existing reference allocation path.
const EXPLICIT: &str = r#"
define class Point as
    pub final int x;
    pub final int y;
    constructor(int a, int b) {
        self.x = a;
        self.y = b;
    }
    pub sum(): int { return self.x + self.y; }
end
pub fnc main(): int {
    let p: Point = new Point(40, 2);
    return p.sum();
}
"#;
#[test]
fn contextual_new_reuses_existing_checked_constructor_and_emitter() {
    let explicit = atto_compiler::compile(EXPLICIT).expect("existing constructor");
    let contextual = EXPLICIT.replace("new Point(40, 2)", "new {x:40,y:2}");
    assert_eq!(atto_compiler::compile(&contextual).expect("contextual constructor"), explicit);
    let reordered = contextual.replace("x:40,y:2", "y:2,x:40");
    assert_eq!(atto_compiler::compile(&reordered).expect("inert reordered fields"), explicit);
    let type_first = contextual.replace("p: Point", "Point p");
    assert_eq!(atto_compiler::compile(&type_first).expect("type-first constructor"), explicit);
}
#[test]
fn contextual_new_rejects_unproved_categories_and_fields() {
    let good = EXPLICIT.replace("new Point(40, 2)", "new {x:40,y:2}");
    for bad in [
        good.replace("p: Point", "p"),
        good.replace("x:40,y:2", "x:40"),
        good.replace("x:40,y:2", "x:40,x:2"),
        good.replace("x:40,y:2", "x:40,z:2"),
        good.replace("x:40,y:2", "x:true,y:2"),
        good.replace("x:40,y:2", "x:40,y:2,z:3"),
        good.replace("pub final int x", "private final int x"),
        good.replace("self.x = a;", "self.x = a + 1;"),
        good.replace("new {x:40,y:2}", "new {x:40,y:2} with actor_heap"),
    ] {
        assert!(atto_compiler::compile(&bad).is_err(), "{bad}");
    }
}

#[test]
fn reordered_expressions_use_nonlexable_typed_temporaries_in_source_order() {
    use atto_compiler::{ast::{ExprKind, Stmt, Type}, lexer, parser::Parser};
    let source = EXPLICIT.replace("new Point(40, 2)", "new {y:1+1,x:39+1}");
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let main = parsed.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Bind { name: first, ty: Some(Type::Int), init: first_init, .. } = &main.body[0] else { panic!("first temp"); };
    let Stmt::Bind { name: second, ty: Some(Type::Int), init: second_init, .. } = &main.body[1] else { panic!("second temp"); };
    assert!(first.starts_with('\0') && second.starts_with('\0') && first != second);
    assert!(matches!(&first_init.kind, ExprKind::Binary(..)));
    assert!(matches!(&second_init.kind, ExprKind::Binary(..)));
    let Stmt::Bind { init, .. } = &main.body[2] else { panic!("final binding"); };
    let ExprKind::New(_, args) = &init.kind else { panic!("ordinary new"); };
    assert!(matches!(&args[0].kind, ExprKind::Var(n) if n == second));
    assert!(matches!(&args[1].kind, ExprKind::Var(n) if n == first));
    assert!(atto_compiler::compile(&source).is_ok());
    assert!(atto_compiler::compile(&source.replace("y:1+1", "y:true")).is_err());
}
