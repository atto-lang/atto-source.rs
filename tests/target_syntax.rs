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
        mtd (mut self: Foo1) set(int amount): void {}
        mtd (self: Foo2) get(): int {}
        mtd (Foo1) get_done(): void {}
        mtd (Foo2 _) get_done(): void {}");
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
    reject("declare actor Foo; mtd (Foo self) done(): void {}", "E_SYNTAX");
    reject("declare actor Foo; mtd (Foo self) test(int val): void {}", "E_SYNTAX");
    reject("declare actor Foo; mtd (Foo mut self) foo(): void {}", "E_SYNTAX");
    let long = format!("declare actor A; mtd (A self) call({}): void {{}}",
        (0..33).map(|n| format!("int p{n}")).collect::<Vec<_>>().join(","));
    reject(&long, "E_SOURCE_LIMIT");
}

#[test]
fn receiver_header_resolution_is_nominal_and_namespace_scoped() {
    use atto_compiler::target_syntax::resolve_receiver_header;
    use atto_compiler::Pos;
    let p = parse("use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        mtd (Foo1 self) foo(): int { return 10; }
        pub mtd (Foo2 self) foo(): int { return 20; }
        mtd (Foo1 self) foo(int value): int { return value; }");
    let pos = Pos { line: 1, col: 1 };
    let left = resolve_receiver_header(&p,Some("workers"),"Foo1","foo",0,false,pos).unwrap();
    let right = resolve_receiver_header(&p,Some("workers"),"Foo2","foo",0,true,pos).unwrap();
    assert_eq!(left.owner,"Foo1");
    assert_eq!(right.owner,"Foo2");
    assert_eq!(left.arity,0);
    assert_eq!(right.arity,0);
    assert_eq!(resolve_receiver_header(&p,Some("workers"),"Foo1","foo",1,false,pos).unwrap().arity,1);
    let failures = [
        (Some("other"),"Foo1","foo",0,false,"E_NAMESPACE"),
        (Some("workers"),"Foo1","foo",0,true,"E_VISIBILITY"),
        (Some("workers"),"Foo2","foo",1,false,"E_UNBOUND"),
        (Some("workers"),"Missing","foo",0,false,"E_UNBOUND"),
    ];
    for (ns, owner, method, arity, external, code) in failures {
        let error=resolve_receiver_header(&p,ns,owner,method,arity,external,pos).unwrap_err();
        assert_eq!(error.code,code,"{error}");
    }
}

#[test]
fn internal_self_calls_bind_only_to_same_nominal_actor() {
    parse("use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        mtd (Foo1 self) foo(): int { return 10; }
        mtd (Foo1 self) other(): int { return self.foo(); }
        mtd (Foo2 self) foo(): int { return 20; }");
    reject("declare actor Foo1; declare actor Foo2;
        mtd (Foo2 self) foo(): void {}
        mtd (Foo1 self) other(): void { self.foo(); }","E_UNBOUND");
    reject("declare actor Foo;
        mtd (Foo self) foo(): void {}
        mtd (Foo self) other(): void { self.foo(1); }","E_UNBOUND");
}

#[test]
fn sealed_state_checks_never_treat_self_fields_as_unsealed() {
    reject("declare actor Foo; mtd (Foo self) foo(): int { return self.count; }","E_SEALED");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.count = 1; }","E_SEALED");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { handle_state(critical-write |s| -> {s.count = 1;}); }","E_SEALED");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(read |s| -> s.count); }","E_SEALED");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> ); }","E_SEALED");
}

#[test]
fn state_callbacks_check_read_write_borrow_and_escape() {
    reject("declare actor Foo; mtd (self: Foo) foo(): void { self.handle_state(critical-write |s| -> { s.count = 5; }); }","E_MUTABILITY");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> { s.count = 5; }); }","E_READONLY");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> { s.count += 5; }); }","E_READONLY");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> { s.count++; }); }","E_READONLY");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> { return s; }); }","E_ESCAPE");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> { val snapshot = s; }); }","E_ESCAPE");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> await foo()); }","E_BORROW");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-write |s| -> { spawn Foo(); }); }","E_BORROW");
    reject("declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-write |s| -> { self.handle_state(critical-read |r| -> r.count); }); }","E_BORROW");
    reject("declare actor Foo; mtd (self: Foo) foo(): int { return self.handle_state(critical-read |s| -> s.map.delete(1)); }","E_UNSUPPORTED");
}
#[test]
fn valid_callback_uses_are_accepted_for_nonexecuting_profile() {
    let s="declare actor Foo;
        mtd (mut self: Foo) add(int value): void {
            self.handle_state(critical-write |s| -> { s.count = s.count + value; });
        }
        mtd (self: Foo) count(): int {
            return self.handle_state(critical-read |s| -> s.count);
        }";
    parse(s);
    assert_eq!(atto_compiler::compile(s).unwrap_err().code,"E_UNSUPPORTED");
}
#[test]
fn local_spawn_capabilities_reject_synchronous_method_calls() {
    let source="declare actor Foo;
        mtd (Foo self) foo(): void {}
        pub fnc main(): int { val first = spawn Foo(); first.foo(); return 0; }";
    reject(source,"E_ISOLATION");
    let safe="declare actor Foo;
        pub fnc main(): int {
            val first = spawn Foo();
            first.send(42);
            return 0;
        }";
    parse(safe);
    assert_eq!(atto_compiler::compile(safe).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn lexical_tokens_prevent_fake_state_reads_from_comments_or_strings() {
    parse("declare actor Foo;
        mtd (mut self: Foo) foo(): void {
            // self.count = 1; await panic();
            val text = \"self.count; s.count++; handle_state(critical-write)\";
            self.handle_state(critical-write |s| -> {s.count = 1;});
        }");
}

#[test]
fn receiver_values_cannot_escape_or_be_aliased_in_target_profile() {
    // Each actor's nominal receiver is sealed; a body cannot stash an
    // untyped `self` and bypass the eventual actor-owned borrow checker.
    for escaped in [
        "val alias = self;",
        "let alias = self;",
        "return self;",
        "send(self);",
    ] {
        let source = format!(
            "use namespace workers;
             declare actor Foo1;
             declare actor Foo2;
             mtd (Foo1 self) run(): void {{ {escaped} }}
             mtd (Foo2 self) run(): void {{}}"
        );
        reject(&source, "E_ESCAPE");
    }
    // A normal same-actor receiver call is not an escape.
    parse("use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        mtd (Foo1 self) ping(): void {}
        mtd (Foo1 self) run(): void { self.ping(); }
        mtd (Foo2 self) ping(): void {}
        mtd (Foo2 self) run(): void { self.ping(); }");
}

#[test]
fn critical_read_denies_prefix_state_mutation() {
    for prefix in ["++", "--"] {
        let source = format!(
            "declare actor Foo;
             mtd (mut self: Foo) run(): void {{
                self.handle_state(critical-read |s| -> {{ {prefix}s.count; }});
             }}"
        );
        reject(&source, "E_READONLY");
    }
    // Mutations are lexically admitted only by an exclusive writer, still
    // requiring full type/borrow checking before any executable lowering.
    parse("declare actor Foo;
        mtd (mut self: Foo) run(): void {
            self.handle_state(critical-write |s| -> { ++s.count; });
        }");
    reject("declare actor Foo;
        mtd (self: Foo) run(): void {
            self.handle_state(critical-write |s| -> { ++s.count; });
        }", "E_MUTABILITY");
}
