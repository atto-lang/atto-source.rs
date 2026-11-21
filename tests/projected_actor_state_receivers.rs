//! Target syntax tests only: no actor guest lowering or state-layout proof.
//! The compiler must reject projected receiver effects until typed lowering
//! retains and verifies the permission footprint.
use atto_compiler::{compile, lexer::lex};
use atto_compiler::target_syntax::{
    parse_target_declarations, ReceiverStateWrite,
};

fn parse(src: &str) -> atto_compiler::target_syntax::TargetHeaders {
    parse_target_declarations(&lex(src).unwrap()).unwrap()
}

fn reject(src: &str, code: &str) {
    let err = parse_target_declarations(&lex(src).unwrap()).unwrap_err();
    assert_eq!(err.code, code, "{err}");
}

#[test]
fn identity_receiver_supports_actor_runtime_operations_without_mutable_state() {
    let src = r#"
use namespace workers;
declare actor Foo1;
declare actor Foo2;
mtd (Foo1 self) foo(): int { return 41; }
mtd (Foo2 self) foo(): int { return 42; }
mtd (Foo1 self) receive(ActorMail<String> mail): void {
    self.send(self.foo());
    self.end();
}
mtd (Foo2 self) receive(ActorMail<String> mail): void {
    self.send(self.foo());
    self.end();
}
"#;
    let h = parse(src);
    assert_eq!(h.receivers.len(), 4);
    assert!(h.receivers.iter().all(|receiver| !receiver.mutable));
    assert!(h.receivers.iter().all(|receiver| receiver.state_write.is_none()));
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED_TYPE",
        "ActorMail generic is recognized but has no verified guest ABI");
}

#[test]
fn projected_state_receiver_records_an_exact_bounded_write_ceiling() {
    let src = r#"
declare actor Counter;
mtd (Counter mut self.state) reset(): void {}
mtd (Counter mut self.state.counter) increment(): void {
    self.handle_state(critical-write |s| -> { s.counter += 1; });
}
mtd (Counter mut self.state.{counter, xyz}) update(): void {
    self.handle_state(critical-write |s| -> { s.counter += 1; s.xyz = 3; });
}
mtd (Counter mut self.state.{counter}) one(): void {}
"#;
    let h = parse(src);
    assert_eq!(h.receivers.len(), 4);
    assert_eq!(h.receivers[0].state_write, Some(ReceiverStateWrite::Whole));
    assert_eq!(h.receivers[1].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into()])));
    assert_eq!(h.receivers[2].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into(), "xyz".into()])));
    assert_eq!(h.receivers[3].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into()])));
    assert!(h.receivers.iter().all(|receiver| receiver.mutable));
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED",
        "a projection cannot silently lower to a whole-state mutable ABI");
}

#[test]
fn malformed_or_overlapping_projection_headers_fail_closed() {
    for (receiver, expected) in [
        ("Counter mut [self.state.counter, self.state.xyz]", "E_SYNTAX"),
        ("Counter mut [self.state.{counter, xyz}]", "E_SYNTAX"),
        ("Counter mut self", "E_SYNTAX"),
        ("Counter mut self.state.", "E_SYNTAX"),
        ("Counter mut self.state.{}", "E_SYNTAX"),
        ("Counter mut self.state.{counter,}", "E_SYNTAX"),
        ("Counter mut self.state.{counter,,xyz}", "E_SYNTAX"),
        ("Counter mut self.state.{counter,counter}", "E_DUPLICATE"),
        ("Counter mut self.state.{counter,xyz,counter}", "E_DUPLICATE"),
        ("Counter mut self.foo", "E_SYNTAX"),
    ] {
        reject(&format!("declare actor Counter; mtd ({receiver}) f(): void {{}}"),
            expected);
    }
    let fields = (0..33).map(|i| format!("f{i}")).collect::<Vec<_>>().join(",");
    reject(&format!("declare actor Counter; mtd (Counter mut self.state.{{{fields}}}) f(): void {{}}"),
        "E_SOURCE_LIMIT");
}

#[test]
fn receiver_footprints_dont_bypass_guards_or_widen_mutation_rights() {
    reject(r#"declare actor Counter;
mtd (Counter mut self.state.counter) bad(): void {
    self.handle_state(critical-write |s| -> { s.other = 3; });
}"#, "E_MUTABILITY");
    reject(r#"declare actor Counter;
mtd (Counter self) bad(): void {
    self.handle_state(critical-write |s| -> { s.counter = 3; });
}"#, "E_MUTABILITY");
    reject(r#"declare actor Counter;
mtd (Counter mut self.state.counter) bad(): void {
    self.state.counter = 3;
}"#, "E_SEALED");
    reject("define class Counter as end mtd (Counter mut self.state.counter) f(): void {}",
        "E_ACTOR");
}

#[test]
fn projected_mutability_does_not_distinguish_method_overloads() {
    reject("declare actor Counter; mtd (Counter self) f(): void {}\n\
            mtd (Counter mut self.state.counter) f(): void {}",
        "E_DUPLICATE");
}

#[test]
fn actor_runtime_operations_reject_bad_arity_and_malformed_nested_arguments() {
    for source in [
        "declare actor Foo; mtd (Foo self) f(): void { self.send(); }",
        "declare actor Foo; mtd (Foo self) f(): void { self.end(1); }",
    ] {
        reject(source, "E_ARITY");
    }
    for source in [
        "declare actor Foo; mtd (Foo self) f(): void { self.send(1,]); }",
        "declare actor Foo; mtd (Foo self) f(): void { self.send(1,); }",
        "declare actor Foo; mtd (Foo self) f(): void { self.send(1,,2); }",
    ] {
        reject(source, "E_SYNTAX");
    }
    // The enclosing method call has an unclosed parenthesis, so the
    // existing bounded parser correctly reports end-of-argument-list.
    reject("declare actor Foo; mtd (Foo self) f(): void { self.send((1,2]); }", "E_EOF");
}

#[test]
fn named_receivers_bind_the_actual_local_identifier() {
    let src = r#"
declare actor Foo;
mtd (Foo self) read_self(): void { self.send(1); self.end(); }
mtd (Foo this) read_this(): void { this.send(1); this.end(); }
mtd (this: Foo) typed_this(): void { this.send(1); }
mtd (Foo mut this.state.counter) increment(): void {
    this.handle_state(critical-write |s| -> { s.counter += 1; });
}
mtd (Foo mut node.state.{counter, xyz}) update(): void {
    node.handle_state(critical-write |s| -> { s.counter += 1; s.xyz += 1; });
}
mtd (Foo mut counter.state.counter) same_field_name(): void {
    counter.handle_state(critical-write |s| -> { s.counter += 1; });
}
mtd (Foo mut this.state) reset(): void {
    this.handle_state(critical-write |s| -> { s.counter = 0; });
}
"#;
    let h = parse(src);
    assert_eq!(h.receivers.len(), 7);
    let bindings = h.receivers.iter().map(|r| r.binding.as_str()).collect::<Vec<_>>();
    assert_eq!(bindings, ["self", "this", "this", "this", "node", "counter", "this"]);
    assert_eq!(h.receivers[3].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into()])));
    assert_eq!(h.receivers[4].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into(), "xyz".into()])));
    assert_eq!(h.receivers[6].state_write, Some(ReceiverStateWrite::Whole));
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED",
        "named actor receivers remain target-only until authenticated guest lowering");
}

#[test]
fn receiver_renaming_does_not_change_the_declared_write_set() {
    let a = parse("declare actor Foo; mtd (Foo mut self.state.{counter, xyz}) update(): void {}");
    let b = parse("declare actor Foo; mtd (Foo mut this.state.{counter, xyz}) update(): void {}");
    assert_eq!(a.receivers[0].owner, b.receivers[0].owner);
    assert_eq!(a.receivers[0].name, b.receivers[0].name);
    assert_eq!(a.receivers[0].arity, b.receivers[0].arity);
    assert_eq!(a.receivers[0].state_write, b.receivers[0].state_write);
    assert_ne!(a.receivers[0].binding, b.receivers[0].binding);
}

#[test]
fn named_receiver_mismatch_and_shadowing_fail_closed() {
    reject(r#"declare actor Foo;
mtd (Foo mut this.state.counter) f(): void {
    self.handle_state(critical-write |s| -> { s.counter = 1; });
}"#, "E_UNBOUND");
    reject(r#"declare actor Foo;
mtd (Foo mut this.state.counter) f(): void {
    this.state.counter = 1;
}"#, "E_SEALED");
    reject(r#"declare actor Foo;
mtd (Foo mut this.state.counter) f(): void {
    this.handle_state(critical-write |this| -> { this.counter = 1; });
}"#, "E_SEALED");
    reject("declare actor Foo; mtd (Foo this) f(int this): void {}", "E_DUPLICATE");
    reject("declare actor Foo; mtd (Foo this) f(): void { let int this = 1; }", "E_DUPLICATE");
    reject("declare actor Foo; mtd (Foo this) f(): void { this.unknown(); }", "E_UNBOUND");
    reject("declare actor Foo; mtd (Foo actor) f(): void {}", "E_SYNTAX");
    let class_src = "define class Foo as end mtd (Foo this) f(): void {}";
    assert_eq!(parse(class_src).receivers[0].binding, "this");
    assert_eq!(compile(class_src).unwrap_err().code, "E_UNSUPPORTED",
        "named class receiver must not silently lower with implicit self");
}

#[test]
fn receiver_name_is_not_an_overload_key() {
    reject("declare actor Foo; mtd (Foo self) f(): void {} mtd (Foo this) f(): void {}",
        "E_DUPLICATE");
}

#[test]
fn named_receiver_internal_method_calls_resolve_under_same_owner() {
    let src = "declare actor Foo; mtd (Foo this) foo(): int { return 4; }\n\
        mtd (Foo this) bar(): void { this.send(this.foo()); }";
    let h = parse(src);
    assert_eq!(h.receivers.len(), 2);
    assert!(h.receivers.iter().all(|r| r.binding == "this"));
}
