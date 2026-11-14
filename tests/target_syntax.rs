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
    reject("declare actor A; mtd (Foo self) call(): void {}", "E_UNBOUND");
    reject("declare actor A; mtd (self A) call(): void {}", "E_SYNTAX");
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

#[test]
fn nominal_and_typed_receiver_spellings_are_equivalent() {
    let p = parse("use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        mtd (Foo1 self) foo(): void {}
        mtd (Foo2 self) foo(): void {}
        mtd (mut self: Foo1) set(int val): void {}
        mtd (self: Foo2) get(): int {}
        mtd (Foo1) done(): void {}
        mtd (Foo2 _) done(): void {}");
    assert_eq!(p.receivers.len(), 6);
    assert_eq!(p.receivers[0].owner, "Foo1");
    assert_eq!(p.receivers[1].owner, "Foo2");
    assert!(!p.receivers[0].mutable);
    assert!(p.receivers[2].mutable);
    assert_eq!(p.receivers[2].arity, 1);
    assert_eq!(p.receivers[4].owner, "Foo1");
    assert_eq!(p.receivers[5].owner, "Foo2");
    reject("declare actor Foo; mtd (Foo self) foo(): void {}
        mtd (self: Foo) foo(): void {}", "E_DUPLICATE");
}

#[test]
fn file_scope_declarations_are_detected_after_ordinary_functions() {
    let source = "pub fnc initial(): int {return 1;}
        declare actor Worker;
        mtd (Worker self) ping(): void {}
        pub fnc main(): int {return initial();}";
    assert_eq!(parse(source).actors, vec!["Worker"]);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
    // A string inside an otherwise executable body is never a declaration.
    let safe = "pub fnc main(): int { stdio.println(\"declare actor F; mtd (F self) ping()\"); return 1; }";
    assert!(atto_compiler::compile(safe).is_ok());
}

#[test]
fn module_imports_are_explicit_and_ordered_after_namespace_header() {
    let p = parse("use namespace storefront;
        import module math from \"./math.atto\";
        declare actor A;
        mtd (A self) run(): void {}");
    assert_eq!(p.namespace.as_deref(), Some("storefront"));
    assert_eq!(p.imports, vec!["math"]);
    reject("use namespace storefront;
        import module math from \"./math.atto\";
        import module math from \"./other.atto\";", "E_DUPLICATE");
    reject("import module math from fake;", "E_SYNTAX");
    reject("declare actor A; import module math from \"./math.atto\";", "E_SYNTAX");
}

#[test]
fn source_limits_and_invalid_receiver_forms_remain_fail_closed() {
    reject("declare actor Foo; mtd (Foo self: Foo) foo(): void {}", "E_SYNTAX");
    reject("declare actor Foo; mtd (Foo mut self) foo(): void {}", "E_SYNTAX");
    let long = format!("declare actor A; mtd (A self) call({}): void {{}}",
        (0..33).map(|n| format!("int p{n}")).collect::<Vec<_>>().join(","));
    reject(&long, "E_SOURCE_LIMIT");
}
