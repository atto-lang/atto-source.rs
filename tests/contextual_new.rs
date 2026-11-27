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
    let type_first = contextual.replace("p: Point", "Point p");
    assert_eq!(atto_compiler::compile(&type_first).expect("type-first constructor"), explicit);
}
#[test]
fn contextual_new_rejects_unproved_categories_and_fields() {
    let good = EXPLICIT.replace("new Point(40, 2)", "new {x:40,y:2}");
    for bad in [
        good.replace("p: Point", "p"),
        good.replace("x:40,y:2", "y:2,x:40"),
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
