
use std::process::Command;
use atto_compiler::{compile, compile_js};

const SOURCE: &str = r#"
define struct Point {
    int x;
    int y;
} with {
    constructor(int x, int y) { self.x = x; self.y = y; }
    mtd sum() -> int { return self.x + self.y; }
    mut mtd shift(int n): void { self.x = self.x + n; }
}
define struct Rectangle {
    Point a;
    Point b;
} with {
    constructor(Point a, Point b) { self.a = a; self.b = b; }
    mtd area() -> int { return (self.b.x-self.a.x)*(self.b.y-self.a.y); }
    mut mtd shift(int n): void { self.a.x = self.a.x + n; }
}
pub fnc main(): int {
    let mut p = Point(1, 2);
    let q = Point(4, 6);
    let mut r = Rectangle(p,q);
    let snapshot = copy! r;
    p.shift(10);
    stdio.println(r.area());
    r.shift(1);
    stdio.println(r.area());
    stdio.println(snapshot.area());
    return 0;
}
"#;

#[test]
fn attached_struct_target_is_not_wasm_disguised_as_javascript() {
    let js = compile_js(SOURCE).expect("checked struct ESM");
    assert!(js.starts_with("export function instantiate"));
    assert!(!js.contains("WebAssembly.Memory"));
    assert_eq!(compile_js(SOURCE).unwrap(), js);
    assert_eq!(&compile(SOURCE).unwrap()[..4], b"\0asm");
    let script = format!("{js}\nlet stdout=''; const g=instantiate({{print_i64:n=>stdout+=String(n),print_newline:()=>stdout+='\\n'}}); if(g.exports.main()!==0n || g.exports.panic_code.value!==0 || stdout!=='12\\n8\\n12\\n' || 'memory' in g.exports) process.exit(1); process.stdout.write(stdout);");
    let result = Command::new("node").args(["--input-type=module", "-e", &script]).output().expect("Node is required for actual ESM guest verification");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(result.stdout, b"12\n8\n12\n");
}

#[test]
fn rejected_structs_do_not_produce_esm() {
    for source in [
        SOURCE.replace("Point a;", "Rectangle a;"),
        SOURCE.replace("self.b = b;", ""),
        SOURCE.replace("let mut r =", "let r ="),
    ] {
        assert!(compile_js(&source).is_err());
        assert!(compile(&source).is_err());
    }
}
