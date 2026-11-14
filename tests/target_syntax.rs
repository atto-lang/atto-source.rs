use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;

fn parse(s: &str) -> atto_compiler::target_syntax::TargetHeaders {
    parse_target_declarations(&lex(s).unwrap()).unwrap()
}
fn reject(s: &str, code: &str) {
    let d = parse_target_declarations(&lex(s).unwrap()).unwrap_err();
    assert_eq!(d.code, code, "{d}");
}
#[test]
fn namespace_and_multiple_actor_receivers_are_distinct() {
    let source = r#"
use namespace workers;
declare actor Foo1;
declare actor Foo2;
mtd (mut self: Foo1) add(int amount): void {
  self.handle_state(critical-write |s| -> { s.count = s.count + amount; });
}
mtd (self: Foo1) count(): int {
  return self.handle_state(critical-read |s| -> s.count);
}
pub mtd (mut self: Foo2) add(int amount): void {
  self.handle_state(critical-write |s| -> { s.last = amount; });
}
pub fnc main(): int {
  val a = spawn Foo1();
  val b = spawn Foo2();
  a.send(1);
  b.send(2);
  return 0;
}"#;
    let p = parse(source);
    assert_eq!(p.namespace.as_deref(), Some("workers"));
    assert_eq!(p.actors, vec!["Foo1", "Foo2"]);
    assert_eq!(p.receivers.len(), 3);
    assert_eq!(p.receivers[0].owner, "Foo1");
    assert_eq!(p.receivers[2].owner, "Foo2");
    assert!(p.receivers[0].mutable);
    assert!(!p.receivers[1].mutable);
    assert!(p.receivers[2].public);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn actor_methods_overload_by_arity_not_receiver_mutability() {
    let p = parse("declare actor A;\nmtd (self: A) call(): void {}\
        \nmtd (self: A) call(int n): void {}");
    assert_eq!(p.receivers.len(), 2);
    reject("declare actor A; mtd (self: A) call(): void {}\
        mtd (mut self: A) call(): void {}", "E_DUPLICATE");
}
#[test]
fn reject_unknown_or_malformed_receiver() {
    reject("declare actor A; mtd (self: B) call(): void {}", "E_UNBOUND");
    reject("declare actor A; mtd (A self) call(): void {}", "E_SYNTAX");
    reject("declare actor A; mtd (self: A) call(int): void {}", "E_SYNTAX");
    reject("declare actor A; mtd (self: A) call(): void", "E_SYNTAX");
}
#[test]
fn reject_duplicate_or_misplaced_namespaces() {
    reject("declare actor A; declare actor A;", "E_DUPLICATE");
    reject("declare actor A; use namespace workers;", "E_SYNTAX");
    reject("use namespace a; use namespace b;", "E_SYNTAX");
    reject("use namespace a.b; declare actor A;", "E_SYNTAX");
}
#[test]
fn reject_unclosed_or_too_deep_bodies() {
    reject("declare actor A; mtd (self: A) call(): void {", "E_EOF");
    let s = format!("declare actor A; mtd (self: A) call(): void {{{}}}",
        "{".repeat(130));
    reject(&s, "E_NESTING");
}
#[test]
fn comments_strings_and_plain_headers_do_not_forge_actors() {
    let p = parse("// declare actor Shadow;\ndeclare actor Real;\
        fnc f(): void { val s = \"declare actor Fake;\"; }");
    assert_eq!(p.actors, vec!["Real"]);
    assert_eq!(atto_compiler::compile("use namespace workers; pub fnc main(): int {return 42;}")
        .unwrap_err().code, "E_UNSUPPORTED");
}
