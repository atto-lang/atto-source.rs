use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;
use atto_compiler::target_typed::{lower_target_signatures, TargetType};
use atto_compiler::{Diagnostic, Pos};

fn typed(source: &str) -> Result<atto_compiler::target_typed::TypedActorModule, Diagnostic> {
    let tokens = lex(source)?;
    let headers = parse_target_declarations(&tokens)?;
    lower_target_signatures(&headers)
}
fn rejects(source: &str, code: &str) {
    let d = typed(source).expect_err(source);
    assert_eq!(d.code, code, "{d}");
    let compiler_d = atto_compiler::compile(source).expect_err(source);
    assert_eq!(compiler_d.code, code, "public compiler: {compiler_d}");
}

#[test]
fn distinct_actors_keep_identical_method_names_and_typed_results() {
    let module = typed(
        "use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        mtd (Foo1 self) foo(): int { return 10; }
        pub mtd (Foo2 self) foo(): bool { return true; }",
    )
    .unwrap();
    assert_eq!(module.actors.len(), 2);
    assert!(module.actors.iter().all(|a| !a.defined));
    let pos = Pos { line: 1, col: 1 };
    let a = module.resolve(Some("workers"), "Foo1", "foo", 0, false, pos).unwrap();
    let b = module.resolve(Some("workers"), "Foo2", "foo", 0, true, pos).unwrap();
    assert_eq!(a.actor, 0);
    assert_eq!(b.actor, 1);
    assert_eq!(a.returns, TargetType::Int);
    assert_eq!(b.returns, TargetType::Bool);
    assert!(!a.body_lowered && !b.body_lowered);
    assert_eq!(
        module.resolve(Some("workers"), "Foo1", "foo", 0, true, pos).unwrap_err().code,
        "E_VISIBILITY"
    );
    assert_eq!(
        module.resolve(Some("foreign"), "Foo1", "foo", 0, false, pos).unwrap_err().code,
        "E_NAMESPACE"
    );
    assert_eq!(
        module.resolve(Some("workers"), "Foo2", "foo", 1, false, pos).unwrap_err().code,
        "E_UNBOUND"
    );
    assert_eq!(module.require_spawnable("Foo1", pos).unwrap_err().code, "E_UNLINKED");
    assert_eq!(
        atto_compiler::compile(
            "use namespace workers;
        declare actor Foo1; mtd (Foo1 self) foo(): int {return 10;}"
        )
        .unwrap_err()
        .code,
        "E_UNSUPPORTED"
    );
}

#[test]
fn parameters_and_actor_capabilities_have_nominal_types() {
    let m = typed(
        "use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        pub mtd (mut self: Foo1) combine(
            ActorRef<Foo2> peer, int count, string label
        ): ActorRef<Foo1> { return peer; }",
    )
    .unwrap();
    assert_eq!(m.receivers.len(), 1);
    let f = &m.receivers[0];
    assert_eq!(f.actor, 0);
    assert_eq!(f.arity, 3);
    assert!(f.mutable && f.public);
    assert_eq!(f.params[0].ty, TargetType::ActorRef(1));
    assert_eq!(f.params[0].name, "peer");
    assert_eq!(f.params[1].ty, TargetType::Int);
    assert_eq!(f.params[2].ty, TargetType::Str);
    assert_eq!(f.returns, TargetType::ActorRef(0));
}

#[test]
fn forward_references_are_resolved_after_all_actor_headers() {
    let m = typed(
        "mtd (Foo1 self) ping(ActorRef<Foo2> peer): void {}
        declare actor Foo2;
        declare actor Foo1;",
    )
    .unwrap();
    assert_eq!(m.receivers[0].params[0].ty, TargetType::ActorRef(0));
    assert_eq!(m.receivers[0].actor, 1);
    assert_eq!(m.receivers[0].returns, TargetType::Void);
}

#[test]
fn only_explicit_actor_refs_can_cross_method_boundaries() {
    rejects("declare actor Foo; mtd (Foo self) bad(Foo other): void {}", "E_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(): Foo {}", "E_TYPE");
    rejects(
        "declare actor Foo; mtd (Foo self) bad(ActorRef<Unknown> peer): void {}",
        "E_UNBOUND",
    );
    rejects("declare actor Foo; mtd (Foo self) bad(): ActorRef<Unknown> {}", "E_UNBOUND");
}
#[test]
fn unknown_and_unsupported_types_fail_before_wasm_emission() {
    rejects("declare actor Foo; mtd (Foo self) bad(Mystery value): void {}", "E_UNKNOWN_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(): Mystery {}", "E_UNKNOWN_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(void arg): void {}", "E_TYPE");
    rejects(
        "declare actor Foo; mtd (Foo self) bad(Option<int> value): void {}",
        "E_UNSUPPORTED_TYPE",
    );
    rejects("declare actor Foo; mtd (Foo self) bad(ActorRef<> value): void {}", "E_SYNTAX");
}
#[test]
fn parameter_names_must_be_unique_and_method_arity_stable() {
    rejects("declare actor Foo; mtd (Foo self) bad(int x, bool x): void {}", "E_DUPLICATE");
    let m = typed(
        "declare actor Foo;
        mtd (Foo self) foo(int value): void {}
        mtd (Foo self) foo(int a, bool b): void {}",
    )
    .unwrap();
    assert_eq!(m.receivers.len(), 2);
    assert_eq!(m.receivers[0].arity, 1);
    assert_eq!(m.receivers[1].arity, 2);
}
