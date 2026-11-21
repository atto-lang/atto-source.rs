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
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED",
        "actor method execution is not implemented by the syntax frontend");
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
