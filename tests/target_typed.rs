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
    let module = typed("use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        mtd (Foo1 self) foo(): int { return 10; }
        pub mtd (Foo2 self) foo(): bool { return true; }").unwrap();
    assert_eq!(module.actors.len(), 2);
    assert!(module.actors.iter().all(|a| !a.defined));
    let pos=Pos {line:1,col:1};
    let a=module.resolve(Some("workers"),"Foo1","foo",0,false,pos).unwrap();
    let b=module.resolve(Some("workers"),"Foo2","foo",0,true,pos).unwrap();
    assert_eq!(a.actor,0);
    assert_eq!(b.actor,1);
    assert_eq!(a.returns,TargetType::Int);
    assert_eq!(b.returns,TargetType::Bool);
    assert!(!a.body_lowered && !b.body_lowered);
    assert_eq!(module.resolve(Some("workers"),"Foo1","foo",0,true,pos)
        .unwrap_err().code,"E_VISIBILITY");
    assert_eq!(module.resolve(Some("foreign"),"Foo1","foo",0,false,pos)
        .unwrap_err().code,"E_NAMESPACE");
    assert_eq!(module.resolve(Some("workers"),"Foo2","foo",1,false,pos)
        .unwrap_err().code,"E_UNBOUND");
    assert_eq!(module.require_spawnable("Foo1",pos).unwrap_err().code,"E_UNLINKED");
    assert_eq!(atto_compiler::compile("use namespace workers;
        declare actor Foo1; mtd (Foo1 self) foo(): int {return 10;}")
        .unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn parameters_and_actor_capabilities_have_nominal_types() {
    let m=typed("use namespace workers;
        declare actor Foo1;
        declare actor Foo2;
        pub mtd (mut self: Foo1) combine(
            ActorRef<Foo2> peer, int count, string label
        ): ActorRef<Foo1> { return peer; }").unwrap();
    assert_eq!(m.receivers.len(),1);
    let f=&m.receivers[0];
    assert_eq!(f.actor,0);
    assert_eq!(f.arity,3);
    assert!(f.mutable && f.public);
    assert_eq!(f.params[0].ty,TargetType::ActorRef(1));
    assert_eq!(f.params[0].name,"peer");
    assert_eq!(f.params[1].ty,TargetType::Int);
    assert_eq!(f.params[2].ty,TargetType::Str);
    assert_eq!(f.returns,TargetType::ActorRef(0));
}

#[test]
fn forward_references_are_resolved_after_all_actor_headers() {
    let m=typed("mtd (Foo1 self) ping(ActorRef<Foo2> peer): void {}
        declare actor Foo2;
        declare actor Foo1;").unwrap();
    assert_eq!(m.receivers[0].params[0].ty,TargetType::ActorRef(0));
    assert_eq!(m.receivers[0].actor,1);
    assert_eq!(m.receivers[0].returns,TargetType::Void);
}

#[test]
fn only_explicit_actor_refs_can_cross_method_boundaries() {
    rejects("declare actor Foo; mtd (Foo self) bad(Foo other): void {}","E_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(): Foo {}","E_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(ActorRef<Unknown> peer): void {}","E_UNBOUND");
    rejects("declare actor Foo; mtd (Foo self) bad(): ActorRef<Unknown> {}","E_UNBOUND");
}
#[test]
fn unknown_and_unsupported_types_fail_before_wasm_emission() {
    rejects("declare actor Foo; mtd (Foo self) bad(Mystery value): void {}","E_UNKNOWN_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(): Mystery {}","E_UNKNOWN_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(void arg): void {}","E_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(Option<int> value): void {}","E_UNSUPPORTED_TYPE");
    rejects("declare actor Foo; mtd (Foo self) bad(ActorRef<> value): void {}","E_SYNTAX");
}
#[test]
fn parameter_names_must_be_unique_and_method_arity_stable() {
    rejects("declare actor Foo; mtd (Foo self) bad(int x, bool x): void {}","E_DUPLICATE");
    let m=typed("declare actor Foo;
        mtd (Foo self) foo(int value): void {}
        mtd (Foo self) foo(int a, bool b): void {}").unwrap();
    assert_eq!(m.receivers.len(),2);
    assert_eq!(m.receivers[0].arity,1);
    assert_eq!(m.receivers[1].arity,2);
}

#[test]
fn concrete_actor_scalar_fields_are_verified_in_typed_state_ir() {
    let source = r#"
use namespace workers;
declare actor Counter;
define actor Counter as
    let int count = 0;
    val int limit = 12;
end
mtd (mut self: Counter) increment(): void {
    self.handle_state(critical-write |state| -> {
        state.count = state.count + 1;
    });
}
mtd (self: Counter) read(): int {
    return self.handle_state(critical-read |state| -> state.limit);
}
"#;
    let module = typed(source).unwrap();
    assert_eq!(module.state_uses.len(), 3);
    assert_eq!(module.state_uses[0].field, "count");
    assert_eq!(module.state_uses[0].field_ty, Some(TargetType::Int));
    assert!(module.state_uses[0].mutation);
    assert!(module.state_uses[0].exclusive);
    assert_eq!(module.state_uses[1].field_ty, Some(TargetType::Int));
    assert!(!module.state_uses[1].mutation);
    assert_eq!(module.state_uses[2].field, "limit");
    assert_eq!(module.state_uses[2].field_ty, Some(TargetType::Int));
    assert!(!module.state_uses[2].exclusive);
    assert!(!module.actors[0].defined);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn concrete_state_fields_reject_unbound_immutable_or_unproved_projections() {
    let prefix = r#"
declare actor Counter;
define actor Counter as
    let int count = 0;
    val int limit = 12;
    let string label = "hello";
end
"#;
    for (body, code) in [
        ("self.handle_state(critical-read |s| -> s.missing);", "E_UNBOUND"),
        ("self.handle_state(critical-write |s| -> { s.limit = 1; });", "E_READONLY"),
        ("self.handle_state(critical-write |s| -> { ++s.limit; });", "E_READONLY"),
        ("self.handle_state(critical-read |s| -> s.label);", "E_BORROW"),
    ] {
        rejects(&format!("{prefix} mtd (mut self: Counter) run(): void {{ {body} }}"), code);
    }
}

#[test]
fn forwarded_actor_state_stays_explicitly_unverified_until_linked() {
    let src = "declare actor Foo;
        mtd (Foo self) read(): int {
            return self.handle_state(critical-read |s| -> s.count);
        }";
    let module = typed(src).unwrap();
    assert_eq!(module.state_uses.len(), 1);
    assert_eq!(module.state_uses[0].field_ty, None);
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn state_field_resolution_is_nominal_for_multiple_actors() {
    let source = r#"
declare actor Foo1;
declare actor Foo2;
define actor Foo1 as let int count = 0; end
define actor Foo2 as let bool ready = true; end
mtd (Foo1 self) read(): int {
    return self.handle_state(critical-read |s| -> s.count);
}
mtd (Foo2 self) read(): bool {
    return self.handle_state(critical-read |s| -> s.ready);
}
"#;
    let module = typed(source).unwrap();
    assert_eq!(module.state_uses.len(), 2);
    assert_eq!(module.state_uses[0].field_ty, Some(TargetType::Int));
    assert_eq!(module.state_uses[1].field_ty, Some(TargetType::Bool));
    rejects(&source.replace("s.ready", "s.count"), "E_UNBOUND");
}

#[test]
fn sealed_receive_hook_is_rejected_even_when_actor_is_only_forward_declared() {
    rejects("declare actor Foo; mtd (Foo self) receive(): void {}", "E_SIGNATURE");
    rejects("use namespace workers; declare actor Foo;
        mtd (Foo self) receive(int message): void {}", "E_SIGNATURE");
    // A differently named ordinary receiver remains a syntax-only method.
    let source = "declare actor Foo; mtd (Foo self) on_message(int message): void {}";
    assert!(typed(source).is_ok());
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn typed_lowerer_revalidates_critical_lease_invariants_even_for_imported_headers() {
    let source = "declare actor Counter;
        define actor Counter as let int count = 0; end
        mtd (mut self: Counter) run(): int {
            return self.handle_state(critical-read |s| -> s.count);
        }";
    let mut headers = parse_target_declarations(&lex(source).unwrap()).unwrap();
    assert_eq!(headers.state_uses.len(), 1);
    assert!(!headers.state_uses[0].mutation);
    assert!(!headers.state_uses[0].exclusive);
    headers.state_uses[0].mutation = true;
    assert_eq!(lower_target_signatures(&headers).unwrap_err().code, "E_READONLY");
    headers.state_uses[0].mutation = false;
    headers.state_uses[0].exclusive = true;
    headers.receivers[0].mutable = false;
    assert_eq!(lower_target_signatures(&headers).unwrap_err().code, "E_MUTABILITY");
}

#[test]
fn nominal_receiver_effects_propagate_through_indirect_calls() {
    let src = r#"
use namespace workers;
declare actor Alpha;
declare actor Beta;
define actor Alpha as let int count = 0; end
define actor Beta as let bool ready = true; end
mtd (mut self: Alpha) update(): void {
    self.handle_state(critical-write |s| -> { s.count = s.count + 1; });
}
mtd (mut self: Alpha) dispatch(): void { self.update(); }
mtd (mut self: Alpha) entry(): void { self.dispatch(); }
mtd (Beta self) update(): bool {
    return self.handle_state(critical-read |s| -> s.ready);
}
mtd (Beta self) entry(): bool { return self.update(); }
"#;
    let m = typed(src).unwrap();
    let pos = Pos {line: 1,col: 1};
    let writer = m.resolve(Some("workers"),"Alpha","entry",0,false,pos).unwrap();
    assert!(writer.effects.reads_state);
    assert!(writer.effects.writes_state);
    assert!(!writer.effects.may_suspend);
    assert!(!writer.effects.unproved_call);
    assert!(!writer.body_lowered);
    let reader = m.resolve(Some("workers"),"Beta","entry",0,false,pos).unwrap();
    assert!(reader.effects.reads_state);
    assert!(!reader.effects.writes_state,
        "same-named receiver in another actor must not acquire Alpha's write effects");
}

#[test]
fn recursive_actor_call_graph_reaches_a_bounded_effect_fixed_point() {
    let src = r#"
declare actor Runner;
mtd (Runner self) a(): void { self.b(); }
mtd (Runner self) b(): void { self.a(); self.c(); }
mtd (Runner self) c(): void { await pending(); }
"#;
    let m=typed(src).unwrap();
    for method in ["a","b","c"] {
        let effect=&m.resolve(None,"Runner",method,0,false,Pos{line:1,col:1})
            .unwrap().effects;
        assert!(effect.may_suspend,"{method} must inherit transitive await effect");
    }
    assert!(m.resolve(None,"Runner","a",0,false,Pos{line:1,col:1})
        .unwrap().effects.unproved_call);
}

#[test]
fn actor_receiver_effects_are_conservative_for_unresolved_calls_and_exceptions() {
    let src=r#"
declare actor Worker;
mtd (Worker self) unsafe_work(): void {
    val x = compute(1);
    throw Error("bad");
}
mtd (Worker self) wrapper(): void { self.unsafe_work(); }
mtd (Worker self) allocator(): void { val v = new Box(); }
mtd (Worker self) launcher(): void { spawn Worker(); }
"#;
    let m=typed(src).unwrap();
    let get=|name| &m.resolve(None,"Worker",name,0,false,Pos{line:1,col:1})
        .unwrap().effects;
    assert!(get("wrapper").may_unwind);
    assert!(get("wrapper").unproved_call);
    assert!(get("allocator").may_allocate);
    assert!(get("launcher").may_spawn);
    // No proof of safe code generation is obtained from these facts.
    assert!(m.receivers.iter().all(|method| !method.body_lowered));
}
