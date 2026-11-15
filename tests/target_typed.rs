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
        ): ActorRef<Foo2> { return peer; }").unwrap();
    assert_eq!(m.receivers.len(),1);
    let f=&m.receivers[0];
    assert_eq!(f.actor,0);
    assert_eq!(f.arity,3);
    assert!(f.mutable && f.public);
    assert_eq!(f.params[0].ty,TargetType::ActorRef(1));
    assert_eq!(f.params[0].name,"peer");
    assert_eq!(f.params[1].ty,TargetType::Int);
    assert_eq!(f.params[2].ty,TargetType::Str);
    assert_eq!(f.returns,TargetType::ActorRef(1));
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

#[test]
fn scalar_state_assignment_expressions_are_checked_against_concrete_layout() {
    let src=r#"
declare actor Counter;
define actor Counter as
    let int count = 0;
    let bool enabled = false;
end
mtd (mut self: Counter) tick(int amount): void {
    self.handle_state(critical-write |s| -> {
        s.count = (s.count + amount * 2);
        s.enabled = (s.count == 4);
    });
}
"#;
    let module=typed(src).expect("well-typed scalar state lease");
    assert_eq!(module.state_uses.len(), 4);
    let tick=&module.receivers[0];
    assert!(tick.effects.reads_state && tick.effects.writes_state);
    assert!(!tick.body_lowered);
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn scalar_state_write_types_and_numeric_mutations_fail_closed() {
    let prefix=r#"
declare actor Counter;
define actor Counter as
    let int count = 0;
    let bool enabled = false;
end
"#;
    for (body, code) in [
        ("s.count = true;", "E_TYPE"),
        ("s.enabled = 1;", "E_TYPE"),
        ("s.count += false;", "E_TYPE"),
        ("++s.enabled;", "E_TYPE"),
        ("s.enabled += true;", "E_TYPE"),
        ("s.count = s.enabled;", "E_TYPE"),
        ("s.count = missing;", "E_UNBOUND"),
        ("s.count = new Widget();", "E_BORROW"),
        ("s.count = s.count + false;", "E_TYPE"),
    ] {
        let src=format!("{prefix} mtd (mut self: Counter) tick(): void {{
            self.handle_state(critical-write |s| -> {{ {body} }});
        }}");
        rejects(&src, code);
    }
}

#[test]
fn even_empty_critical_callbacks_have_read_or_write_effects() {
    let src=r#"
declare actor Foo;
mtd (Foo self) read(): int {
    return self.handle_state(critical-read |s| -> 1);
}
mtd (mut self: Foo) write(): int {
    return self.handle_state(critical-write |s| -> 1);
}
"#;
    let module=typed(src).unwrap();
    assert!(module.receivers[0].effects.reads_state);
    assert!(!module.receivers[0].effects.writes_state);
    assert!(module.receivers[1].effects.reads_state);
    assert!(module.receivers[1].effects.writes_state);
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn actor_scalar_literal_range_and_numeric_ordering_follow_checked_int_contract() {
    let prefix = "declare actor Counter;
        define actor Counter as let int count = 0; let bool enabled = false; end
        mtd (mut self: Counter) run(): void {
            self.handle_state(critical-write |s| -> {";
    for (body, code) in [
        ("s.count = 9223372036854775808;", "E_INTEGER_RANGE"),
        ("s.count = (9223372036854775808);", "E_INTEGER_RANGE"),
        ("s.enabled = true < false;", "E_TYPE"),
        ("s.enabled = s.enabled >= false;", "E_TYPE"),
    ] {
        rejects(&format!("{prefix} {body} }}); }}"), code);
    }
    for body in [
        "s.count = -9223372036854775808;",
        "s.enabled = true eq false;",
        "s.enabled = s.count < 12;",
    ] {
        let source = format!("{prefix} {body} }}); }}");
        let module = typed(&source).expect("well-typed checked scalar");
        assert!(!module.receivers[0].body_lowered);
        assert_eq!(atto_compiler::compile(&source).unwrap_err().code, "E_UNSUPPORTED");
    }
}

#[test]
fn checked_arithmetic_panic_effects_propagate_without_claiming_runtime_lease_safety() {
    let source = r#"
declare actor Counter;
define actor Counter as let int count = 0; end
mtd (mut self: Counter) increment(): void {
    self.handle_state(critical-write |s| -> { s.count = s.count + 1; });
}
mtd (mut self: Counter) outer(): void { self.increment(); }
mtd (Counter self) literal(): int { return 42; }
mtd (Counter self) explicit_panic(): void { panic(Error("x")); }
"#;
    let module = typed(source).unwrap();
    let pos = Pos {line:1,col:1};
    for method in ["increment", "outer", "explicit_panic"] {
        let effect = &module.resolve(None,"Counter",method,0,false,pos).unwrap().effects;
        assert!(effect.may_panic, "missing fatal panic effect in {method}");
    }
    assert!(!module.resolve(None,"Counter","literal",0,false,pos)
        .unwrap().effects.may_panic);
    assert!(!module.resolve(None,"Counter","explicit_panic",0,false,pos)
        .unwrap().effects.may_unwind,
        "panic must not be treated as catchable throw/raise");
}

#[test]
fn state_projection_address_taking_is_forbidden_under_both_leases() {
    let prefix = "declare actor Foo;
        define actor Foo as let int count = 0; end
        mtd (mut self: Foo) foo(): void {";
    for lease in ["critical-read", "critical-write"] {
        let source = format!("{prefix}
            self.handle_state({lease} |s| -> {{ val leaked = &s.count; }});
        }}");
        rejects(&source, "E_BORROW");
    }
}

#[test]
fn typed_receiver_returns_are_nominal_and_never_claim_executable_lowering() {
    let src = r#"
use namespace workers;
declare actor Foo1;
declare actor Foo2;
define actor Foo1 as let int count = 1; end
define actor Foo2 as let bool ready = true; end
mtd (Foo1 self) value(): int { return 10; }
mtd (Foo1 self) forwarded(): int { return self.value(); }
mtd (Foo1 self) load(): int {
    return self.handle_state(critical-read |s| -> s.count);
}
mtd (Foo2 self) value(): bool { return true; }
mtd (Foo2 self) load(): bool {
    return self.handle_state(critical-read |s| -> s.ready);
}
"#;
    let m = typed(src).unwrap();
    assert_eq!(m.body_returns.len(), 5);
    for return_fact in &m.body_returns {
        assert!(return_fact.result.is_some(), "simple return should have a typed fact");
    }
    assert_eq!(m.body_returns[0].result,Some(TargetType::Int));
    assert_eq!(m.body_returns[1].result,Some(TargetType::Int));
    assert_eq!(m.body_returns[2].result,Some(TargetType::Int));
    assert_eq!(m.body_returns[3].result,Some(TargetType::Bool));
    assert_eq!(m.body_returns[4].result,Some(TargetType::Bool));
    assert!(m.receivers.iter().all(|r| !r.body_lowered),
        "partial typed body IR is not executable receiver lowering");
    assert_eq!(atto_compiler::compile(src).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn known_mismatched_receiver_returns_reject_before_wasm_emission() {
    for source in [
        "declare actor Foo; mtd (Foo self) value(): int { return true; }",
        "declare actor Foo; mtd (Foo self) value(): bool { return 123; }",
        "declare actor Foo; mtd (Foo self) value(): void { return 123; }",
        "declare actor Foo; mtd (Foo self) value(): int { return; }",
        "declare actor Foo; mtd (Foo self) value(bool flag): int { return flag; }",
        "declare actor Foo; mtd (Foo self) one(): bool { return true; }
         mtd (Foo self) two(): int { return self.one(); }",
        "declare actor Foo; declare actor Other;
         mtd (Foo self) other(ActorRef<Other> peer): ActorRef<Foo> { return peer; }",
        "declare actor Foo; define actor Foo as let int count = 1; end
         mtd (Foo self) value(): bool {
             return self.handle_state(critical-read |s| -> s.count);
         }",
    ] {
        rejects(source,"E_TYPE");
    }
}

#[test]
fn unknown_return_expressions_remain_unproved_not_executable() {
    let source = "declare actor Foo;
        mtd (Foo self) complex(int n): int { return n + 1; }";
    let m=typed(source).unwrap();
    assert_eq!(m.body_returns.len(),1);
    assert_eq!(m.body_returns[0].result,None);
    assert!(!m.receivers[0].body_lowered);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn receiver_cfg_proves_all_branches_return_and_preserves_aot_gate() {
    let source = r#"
use namespace workers;
declare actor Alpha;
declare actor Beta;
mtd (Alpha self) choice(bool flag): int {
    if flag {
        return 10;
    } else {
        return 20;
    }
}
mtd (Beta self) choice(bool flag): bool {
    if !flag { return true; } else { return false; }
}
"#;
    let m=typed(source).unwrap();
    assert_eq!(m.receiver_flows.len(),2);
    for flow in &m.receiver_flows {
        assert_eq!(flow.branches,1);
        assert_eq!(flow.returns,2);
        assert!(flow.fully_structured);
        assert_eq!(flow.normal_fallthrough,Some(false));
    }
    assert!(m.receivers.iter().all(|r| !r.body_lowered));
    assert_eq!(atto_compiler::compile(source).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn receiver_cfg_rejects_partially_returning_branches() {
    for source in [
        "declare actor Foo;
         mtd (Foo self) choose(bool flag): int {
             if flag { return 1; } else {}
         }",
        "declare actor Foo;
         mtd (Foo self) choose(bool flag): int {
             if flag { return 1; }
         }",
        "declare actor Foo;
         mtd (Foo self) choose(bool flag): int {
             if flag { return 1; } else if false { return 2; }
         }",
    ] {
        rejects(source,"E_MISSING_RETURN");
    }
    rejects("declare actor Foo;
        mtd (Foo self) choose(int n): int {
            if n { return 1; } else { return 2; }
        }", "E_TYPE");
}

#[test]
fn receiver_cfg_does_not_invent_proofs_for_unlowered_loops() {
    let source="declare actor Foo;
        mtd (Foo self) choose(bool flag): int {
            while flag { return 1; }
        }";
    let m=typed(source).unwrap();
    assert_eq!(m.receiver_flows[0].normal_fallthrough,None);
    assert!(!m.receiver_flows[0].fully_structured);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn receiver_nominal_argument_types_are_checked_for_known_scalars() {
    let base="declare actor Foo;
        mtd (Foo self) use_value(int n,bool enabled): void {}
        mtd (Foo self) caller(int n,bool enabled): void {";
    for args in ["true,enabled", "n,12", "enabled,n"] {
        rejects(&format!("{base} self.use_value({args}); }}"),"E_TYPE");
    }
    let valid=format!("{base} self.use_value(n,(enabled)); }}");
    let m=typed(&valid).unwrap();
    assert!(!m.receivers[1].effects.unproved_call);
    assert_eq!(atto_compiler::compile(&valid).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn actor_ref_call_arguments_keep_nominal_actor_identity() {
    let base=r#"
use namespace workers;
declare actor Foo1;
declare actor Foo2;
mtd (Foo1 self) send_to(ActorRef<Foo1> peer): void {}
"#;
    let invalid=format!("{base}
        mtd (Foo1 self) caller(ActorRef<Foo2> peer): void {{
            self.send_to(peer);
        }}");
    rejects(&invalid,"E_TYPE");
    let valid=format!("{base}
        mtd (Foo1 self) caller(ActorRef<Foo1> peer): void {{
            self.send_to(peer);
        }}");
    assert!(typed(&valid).is_ok());
}

#[test]
fn unknown_call_arguments_remain_unproved_and_not_executable() {
    let source="declare actor Foo;
        mtd (Foo self) use_value(int n): void {}
        mtd (Foo self) caller(): void { self.use_value(calculate()); }";
    let m=typed(source).unwrap();
    assert!(m.receivers[1].effects.unproved_call);
    assert_eq!(m.receiver_flows[1].normal_fallthrough,None);
    assert!(m.receivers.iter().all(|r| !r.body_lowered));
    assert_eq!(atto_compiler::compile(source).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn actor_ownership_rejects_double_take_and_use_after_move() {
    let prefix="declare actor Foo;
        mtd (Foo self) consume(ActorRef<Foo> peer): void {}
        mtd (Foo self) run(ActorRef<Foo> peer): void {";
    for body in [
        "rt take peer; rt take peer;",
        "rt take peer; self.consume(peer);",
        "val owned = rt take peer; self.consume(peer);",
        "rt take peer; val previous = peer;",
    ] {
        rejects(&format!("{prefix} {body} }}"),"E_MOVE");
    }
}

#[test]
fn actor_ownership_joins_branch_moves_and_respects_abrupt_exit_paths() {
    let prefix="declare actor Foo;
        mtd (Foo self) consume(ActorRef<Foo> peer): void {}
        mtd (Foo self) run(bool flag,ActorRef<Foo> peer): void {";
    rejects(&format!("{prefix}
        if flag {{ rt take peer; }} else {{}}
        self.consume(peer); }}"),"E_MOVE");
    let safe=format!("{prefix}
        if flag {{ rt take peer; return; }} else {{}}
        self.consume(peer); }}");
    let m=typed(&safe).expect("a terminating moved branch cannot poison a live path");
    assert_eq!(m.receiver_borrows[1].branch_joins,1);
    assert_eq!(m.receiver_borrows[1].moves,1);
    assert!(!m.receivers[1].body_lowered);
    assert_eq!(atto_compiler::compile(&safe).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn actor_ownership_denies_shared_mutable_conflicts_and_moves_under_loan() {
    let prefix="declare actor Foo;
        mtd (Foo self) run(ActorRef<Foo> peer): void {";
    rejects(&format!("{prefix} val a = rt borrow peer; rt take peer; }}"),"E_BORROW");
    rejects(&format!("{prefix}
        let value = 10;
        val read_alias = rt borrow value;
        val write_alias = rt borrow mut value; }}"),"E_BORROW");
    rejects(&format!("{prefix}
        let value = 10;
        val write_alias = rt borrow mut value;
        val read_alias = rt borrow value; }}"),"E_BORROW");
    rejects(&format!("{prefix}
        let value = 10;
        val write_alias = rt borrow mut value;
        val copy_value = value; }}"),"E_BORROW");
    rejects(&format!("{prefix}
        val write_alias = rt borrow mut peer; }}"),"E_MUTABILITY");
}

#[test]
fn actor_ownership_denies_suspend_unwind_and_unproved_calls_while_borrowed() {
    let prefix="declare actor Foo;
        mtd (Foo self) consume(ActorRef<Foo> peer): void {}
        mtd (Foo self) run(ActorRef<Foo> peer): void {";
    for body in [
        "val ref_one = rt borrow peer; await pending();",
        "val ref_one = rt borrow peer; self.consume(peer);",
        "val ref_one = rt borrow peer; panic(Error(\"bad\"));",
        "val ref_one = rt borrow peer; throw Error(\"bad\");",
        "val ref_one = rt borrow peer; return;",
    ] {
        rejects(&format!("{prefix} {body} }}"),"E_BORROW");
    }
}

#[test]
fn actor_ownership_prevents_return_and_call_escape_of_borrow_alias() {
    let prefix="declare actor Foo;
        mtd (Foo self) consume(ActorRef<Foo> peer): void {}
        mtd (Foo self) run(ActorRef<Foo> peer): void {";
    rejects(&format!("{prefix}
        val borrowed = rt borrow peer; self.consume(borrowed); }}"),"E_ESCAPE");
    rejects(&format!("{prefix}
        val borrowed = rt borrow peer; return borrowed; }}"),"E_ESCAPE");
}

#[test]
fn actor_ownership_releases_lexical_borrow_on_inner_scope_exit() {
    let source="declare actor Foo;
        mtd (Foo self) run(ActorRef<Foo> peer): void {
            { val alias = rt borrow peer; }
            rt take peer;
        }";
    let m=typed(source).expect("inner scope exit releases the lexical loan");
    let facts=&m.receiver_borrows[0];
    assert_eq!(facts.loans,1);
    assert_eq!(facts.moves,1);
    assert_eq!(facts.lexical_loan_releases,1);
    assert_eq!(facts.branch_joins,0);
    assert!(facts.ownership_preflight_complete,
        "the bounded ownership subset proves lexical release, not executable code");
    assert!(!m.receivers[0].body_lowered);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn actor_ownership_opaque_loops_never_contribute_an_executable_proof() {
    let src="declare actor Foo;
        mtd (Foo self) run(bool flag,ActorRef<Foo> peer): void {
            while flag { rt take peer; }
        }";
    let m=typed(src).expect("unsupported flow is explicitly unproved");
    assert!(!m.receiver_borrows[0].ownership_preflight_complete);
    assert!(!m.receivers[0].body_lowered);
    assert_eq!(atto_compiler::compile(src).unwrap_err().code,"E_UNSUPPORTED");
}
