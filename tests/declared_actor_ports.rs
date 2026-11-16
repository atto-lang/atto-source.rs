//! Typed nominal actor header compatibility against the existing namespace
//! target parser and conservative declaration/definition linker.
//! These tests prove source metadata; they DO NOT prove executable actors.
use atto_compiler::ast::Type;
use atto_compiler::check::check_target_actor_interface;
use atto_compiler::lexer::lex;
use atto_compiler::parser::Parser;
use atto_compiler::target_syntax::parse_target_declarations;
use atto_compiler::target_typed::{build_actor_program, lower_target_signatures};
use atto_compiler::Pos;

fn target(source: &str) -> Result<(atto_compiler::target_syntax::TargetHeaders,
    atto_compiler::target_typed::TypedActorModule), atto_compiler::Diagnostic> {
    let tokens = lex(source)?;
    let headers = parse_target_declarations(&tokens)?;
    let module = lower_target_signatures(&headers)?;
    let ast = build_actor_program(&headers, &module);
    check_target_actor_interface(&ast)?;
    Ok((headers, module))
}
fn diagnostic(source: &str) -> &'static str {
    let tokens = lex(source).unwrap();
    match parse_target_declarations(&tokens).and_then(|h| {
        let module = lower_target_signatures(&h)?;
        check_target_actor_interface(&build_actor_program(&h, &module))
    }) {
        Ok(()) => panic!("expected invalid declaration"),
        Err(err) => err.code,
    }
}

#[test]
fn canonical_untyped_declare_actor_is_private_nominal_metadata() {
    let (h, m) = target("declare actor Foo;").unwrap();
    assert_eq!(h.actors, vec!["Foo"]);
    assert_eq!(h.actor_ports.len(), 1);
    let port = &h.actor_ports[0];
    assert_eq!(port.name, "Foo");
    assert!(!port.public);
    assert!(port.receives.is_none() && port.emits.is_none() && port.sends.is_none());
    assert!(!m.actors[0].has_definition);
    assert_eq!(m.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
    assert_eq!(atto_compiler::compile("declare actor Foo;").unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn requested_x_and_y_actor_roles_roundtrip_as_nominal_type_spellings() {
    let (h, m) = target("declare actor Foo receives x emits y;").unwrap();
    assert_eq!(h.actor_ports[0].receives.as_deref(), Some("x"));
    assert_eq!(h.actor_ports[0].emits.as_deref(), Some("y"));
    assert_eq!(m.actors[0].receives.as_deref(), Some("x"));
    assert_eq!(m.actors[0].emits.as_deref(), Some("y"));
    assert!(m.actors[0].sends.is_none());
    assert_eq!(atto_compiler::compile("declare actor Foo receives x emits y;")
        .unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn public_declaration_with_implements_and_all_three_roles_survives_linker_ast() {
    let source = "declare pub actor Foo implements ActorXYZ receives int emits bool sends int;";
    let (h, m) = target(source).unwrap();
    let p = build_actor_program(&h, &m);
    let port = &p.actor_interface.unwrap().ports[0];
    assert!(port.public);
    assert_eq!(port.receives.as_deref(), Some("int"));
    assert_eq!(port.emits.as_deref(), Some("bool"));
    assert_eq!(port.sends.as_deref(), Some("int"));
    assert_eq!(port.implemented_contracts, vec!["ActorXYZ"]);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn sends_and_emits_order_both_parse_but_remain_independent() {
    let (_, a) = target("declare actor Foo receives int emits bool sends int;").unwrap();
    let (_, b) = target("declare actor Foo receives int sends int emits bool;").unwrap();
    assert_eq!(a.actors[0].receives, b.actors[0].receives);
    assert_eq!(a.actors[0].emits, b.actors[0].emits);
    assert_eq!(a.actors[0].sends, b.actors[0].sends);
}

#[test]
fn sends_only_never_derives_a_pipe_output_capability() {
    let (_, m) = target("declare actor Foo sends int;").unwrap();
    assert_eq!(m.actors[0].sends.as_deref(), Some("int"));
    assert_eq!(m.actors[0].emits, None);
}

#[test]
fn historical_private_and_public_nominal_declarations_still_work() {
    let (h, _) = target("declare private actor Foo; declare pub actor Bar;").unwrap();
    assert!(!h.actor_ports[0].public);
    assert!(h.actor_ports[1].public);
    assert_eq!(h.actors, vec!["Foo", "Bar"]);
}

#[test]
fn typed_declaration_links_to_matching_inline_definition_without_enabling_guest() {
    let source = "use namespace workers;
        declare actor Foo receives int emits bool;
        define actor Foo receives int emits bool as
           receive(ActorMail<int> mail): void { return; }
        end";
    let (h, m) = target(source).unwrap();
    assert_eq!(h.definitions.len(), 1);
    assert_eq!(h.definitions[0].declared_receives, Some(Type::Int));
    assert_eq!(h.definitions[0].declared_emits, Some(Type::Bool));
    assert!(m.actors[0].has_definition);
    assert!(!m.actors[0].defined);
    assert_eq!(m.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn forward_receives_matches_concrete_receive_handler_without_redundant_clause() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo as
           receive(ActorMail<int> mail): void { return; }
        end";
    assert!(target(source).is_ok());
}

#[test]
fn mismatched_declared_and_concrete_receives_fail_at_linking() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo receives bool as
           receive(ActorMail<bool> mail): void { return; }
        end";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn mismatched_declared_and_concrete_emits_fail_at_linking() {
    let source = "use namespace workers;
        declare actor Foo emits int;
        define actor Foo emits bool as
           receive(ActorMail<int> mail): void { return; }
        end";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn mismatched_declared_and_concrete_sends_fail_at_linking() {
    let source = "use namespace workers;
        declare actor Foo sends bool;
        define actor Foo sends int as
           receive(ActorMail<int> mail): void { return; }
        end";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn inline_actor_with_receives_preserves_the_normal_parser_and_handler_type() {
    let source = "define actor Foo receives int as
        receive(ActorMail<int> mail): void { return; }
        end
        pub fnc main(): int { return 0; }";
    let parsed = Parser::new(lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(parsed.actors[0].declared_receives, Some(Type::Int));
    assert!(atto_compiler::compile(source).is_ok(),
        "matching receives must not disable preexisting concrete actor execution");
}

#[test]
fn mismatched_inline_actor_receive_header_fails_without_guest_output() {
    let source = "define actor Foo receives bool as
        receive(ActorMail<int> mail): void { return; }
        end
        pub fnc main(): int { return 0; }";
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_TYPE");
}

#[test]
fn concrete_inline_actor_sends_and_emits_are_parsed_but_effects_remain_closed() {
    for source in [
        "define actor Foo sends int as end pub fnc main(): int { return 0; }",
        "define actor Foo emits int as end pub fnc main(): int { return 0; }",
    ] {
        assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
    }
}

#[test]
fn duplicate_and_out_of_order_actor_roles_reject_in_namespace_profile() {
    for source in [
        "declare actor Foo receives int receives bool;",
        "declare actor Foo emits int emits bool;",
        "declare actor Foo sends int sends bool;",
        "declare actor Foo emits bool receives int;",
        "declare actor Foo sends int receives bool;",
    ] {
        assert_eq!(diagnostic(source), "E_PIPE_CONTRACT", "{source}");
    }
}

#[test]
fn no_pub_declare_or_define_actor_semicolon_alias() {
    assert_eq!(diagnostic("pub declare actor Foo;"), "E_ACTOR");
    let error = atto_compiler::compile("define actor Foo;").unwrap_err();
    assert_eq!(error.code, "E_ACTOR");
}

#[test]
fn duplicate_nominal_contracts_and_actor_declarations_fail_closed() {
    assert_eq!(diagnostic("declare actor Foo implements X, X;"), "E_DUPLICATE");
    assert_eq!(diagnostic("declare actor Foo; declare actor Foo;"), "E_DUPLICATE");
}

#[test]
fn unlinked_declared_actor_cannot_acquire_runtime_authority_by_spawning() {
    let source = "declare actor Foo receives int emits int;
        pub fnc main(): int { val f = spawn Foo(); return 0; }";
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn old_declared_actor_plus_concrete_target_linker_still_accepts_actor_mtd() {
    let source = "use namespace workers;
        declare actor Foo;
        define actor Foo as
           receive(ActorMail<int> mail): void { return; }
        end
        mtd (Foo self) foo(): int { return 10; }";
    let (h, m) = target(source).unwrap();
    assert!(m.actors[0].has_definition);
    assert_eq!(h.receivers.len(), 1);
}

#[test]
fn declared_and_concrete_headers_cannot_hide_a_different_mail_handler_type() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo receives int as
           receive(ActorMail<bool> mail): void { return; }
        end";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn typed_declaration_plus_empty_definition_is_not_a_verified_receiver() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo as end";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn exact_out_of_line_receive_example_satisfies_declared_input_signature_metadata() {
    let source = "declare actor Foo receives Input emits Output;
        mtd (Foo self) receive(ActorMail<Input> m): void {
        }";
    let (headers, typed) = target(source).unwrap();
    assert_eq!(headers.actor_ports[0].receives.as_deref(), Some("Input"));
    assert_eq!(headers.actor_ports[0].emits.as_deref(), Some("Output"));
    assert!(typed.receivers.is_empty(), "receive is not callable as an ordinary mtd");
    assert_eq!(typed.mailbox_ingresses.len(), 1);
    assert!(typed.actors[0].inbox_witnessed);
    assert!(!typed.actors[0].emits_witnessed);
    assert_eq!(typed.mailbox_ingresses[0].actor, 0);
    assert_eq!(typed.mailbox_ingresses[0].payload, "Input");
    let program = build_actor_program(&headers, &typed);
    let iface = program.actor_interface.unwrap();
    assert!(iface.methods.is_empty());
    assert_eq!(iface.mailbox_ingresses.len(), 1);
    assert_eq!(iface.mailbox_ingresses[0].payload, "Input");
    assert_eq!(typed.resolve(None, "Foo", "receive", 1, false, Pos::default())
        .unwrap_err().code, "E_UNBOUND", "sealed ingress is not a public method");
    assert_eq!(typed.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn private_attached_receiver_must_match_nominal_inbox_type() {
    let source = "declare actor Foo receives Input emits Output;
        mtd (Foo self) receive(ActorMail<Other> m): void {}";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn attached_receiver_requires_exact_actor_mail_envelope() {
    for source in [
        "declare actor Foo receives int; mtd (Foo self) receive(int m): void {}",
        "declare actor Foo receives int; mtd (Foo self) receive(): void {}",
        "declare actor Foo receives int; mtd (Foo self) receive(ActorMail<int> m, int extra): void {}",
        "declare actor Foo receives int; mtd (Foo self) receive(ActorRef<Foo> m): void {}",
        "declare actor Foo receives int; mtd (Foo self) receive(ActorMail<int> m): int { return 0; }",
    ] {
        assert_eq!(diagnostic(source), "E_SIGNATURE", "{source}");
    }
}

#[test]
fn private_receive_ingress_cannot_be_published_or_borrowed_mutably() {
    for source in [
        "declare pub actor Foo receives int; pub mtd (Foo self) receive(ActorMail<int> m): void {}",
        "declare actor Foo receives int; mtd (mut self: Foo) receive(ActorMail<int> m): void {}",
    ] {
        assert_eq!(diagnostic(source), "E_SIGNATURE", "{source}");
    }
}

#[test]
fn double_attached_receiver_is_not_a_multicast_or_overload() {
    let source = "declare actor Foo receives int;
        mtd (Foo self) receive(ActorMail<int> m): void {}
        mtd (Foo self) receive(ActorMail<int> n): void {}";
    assert_eq!(diagnostic(source), "E_DUPLICATE");
}

#[test]
fn inline_and_attached_receiver_cannot_both_claim_one_mailbox() {
    let source = "declare actor Foo receives int;
        define actor Foo as
            receive(ActorMail<int> m): void { return; }
        end
        mtd (Foo self) receive(ActorMail<int> m): void {}";
    assert_eq!(diagnostic(source), "E_DUPLICATE");
}

#[test]
fn out_of_line_receive_can_fill_empty_concrete_actor_inbox_witness() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo as end
        mtd (Foo self) receive(ActorMail<int> m): void {}";
    let (headers, typed) = target(source).unwrap();
    assert!(typed.actors[0].has_definition);
    assert_eq!(typed.mailbox_ingresses.len(), 1);
    assert!(headers.definitions[0].receive.is_none());
    assert_eq!(typed.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
}

#[test]
fn actor_receive_cannot_attach_to_an_undeclared_owner() {
    assert_eq!(diagnostic("declare actor Foo receives int;
        mtd (Bar self) receive(ActorMail<int> m): void {}"), "E_UNBOUND");
}

#[test]
fn actor_mail_envelope_cannot_leak_through_unrelated_actor_method() {
    let source = "declare actor Foo receives int;
        mtd (Foo self) inspect(ActorMail<int> m): void {}";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn receive_input_header_is_metadata_not_verified_without_receiver_implementation() {
    let (h, typed) = target("declare actor Foo receives int emits bool;").unwrap();
    assert!(typed.mailbox_ingresses.is_empty());
    assert_eq!(h.actor_ports[0].receives.as_deref(), Some("int"));
    assert_eq!(typed.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
}

#[test]
fn attached_receive_does_not_satisfy_declared_emits_effect_by_itself() {
    let source = "declare actor Foo receives int emits bool;
        mtd (Foo self) receive(ActorMail<int> m): void {}";
    let (_, typed) = target(source).unwrap();
    assert_eq!(typed.mailbox_ingresses.len(), 1);
    assert_eq!(typed.actors[0].emits.as_deref(), Some("bool"));
    assert!(!typed.actors[0].defined);
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn historical_external_mtd_receive_and_contextual_mtd_share_a_private_sealed_ingress() {
    let plain = "declare actor Foo receives int;
        mtd (Foo self) receive(ActorMail<int> m): void {}";
    let old = "declare actor Foo receives int;
        external mtd (Foo self) receive(ActorMail<int> m): void {}";
    let (_, p) = target(plain).unwrap();
    let (_, e) = target(old).unwrap();
    assert_eq!(p.mailbox_ingresses.len(), 1);
    assert_eq!(e.mailbox_ingresses.len(), 1);
    assert_eq!(p.mailbox_ingresses[0].payload, e.mailbox_ingresses[0].payload);
    assert!(p.receivers.is_empty() && e.receivers.is_empty());
}

#[test]
fn external_ingress_marker_never_grants_an_ordinary_public_method() {
    for source in [
        "declare actor Foo; external mtd (Foo self) ordinary(): void {}",
        "declare pub actor Foo receives int; pub external mtd (Foo self) receive(ActorMail<int> m): void {}",
    ] {
        assert_eq!(diagnostic(source), "E_SIGNATURE", "{source}");
    }
}

#[test]
fn attached_receive_is_never_a_direct_self_method_call() {
    let source = "declare actor Foo receives int;
        mtd (Foo self) receive(ActorMail<int> m): void {}
        mtd (Foo self) retry(): void { self.receive(42); }";
    assert_eq!(diagnostic(source), "E_ISOLATION");
}

#[test]
fn declared_receive_is_sealed_even_when_namespace_lookup_has_matching_arity() {
    use atto_compiler::target_syntax::resolve_receiver_header;
    let source = "use namespace workers;
        declare actor Foo receives int;
        mtd (Foo self) receive(ActorMail<int> m): void {}";
    let (headers, _) = target(source).unwrap();
    let err = resolve_receiver_header(&headers, Some("workers"), "Foo", "receive", 1,
                                      false, Pos::default()).unwrap_err();
    assert_eq!(err.code, "E_ISOLATION");
    let err = resolve_receiver_header(&headers, Some("workers"), "Foo", "receive", 1,
                                      true, Pos::default()).unwrap_err();
    assert_eq!(err.code, "E_ISOLATION");
}

#[test]
fn ordinary_attached_methods_remain_resolvable_without_exposing_receive() {
    use atto_compiler::target_syntax::resolve_receiver_header;
    let source = "declare actor Foo receives int;
        mtd (Foo self) receive(ActorMail<int> m): void {}
        mtd (Foo self) count(): int { return 1; }";
    let (headers, typed) = target(source).unwrap();
    assert_eq!(typed.mailbox_ingresses.len(), 1);
    assert_eq!(typed.receivers.len(), 1);
    assert_eq!(resolve_receiver_header(&headers, None, "Foo", "count", 0,
                                      false, Pos::default()).unwrap().name, "count");
    assert_eq!(resolve_receiver_header(&headers, None, "Foo", "receive", 1,
                                      false, Pos::default()).unwrap_err().code, "E_ISOLATION");
}

#[test]
fn declared_only_inbox_is_unfulfilled_until_a_matching_receiver_is_attached() {
    let (_, without_handler) = target("declare actor Foo receives int emits bool;").unwrap();
    assert!(!without_handler.actors[0].inbox_witnessed);
    assert!(!without_handler.actors[0].emits_witnessed);
    let (_, with_handler) = target("declare actor Foo receives int emits bool;
        mtd (Foo self) receive(ActorMail<int> msg): void {}").unwrap();
    assert!(with_handler.actors[0].inbox_witnessed);
    assert!(!with_handler.actors[0].emits_witnessed);
    assert_eq!(with_handler.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
}

#[test]
fn inline_handler_satisfies_same_nominal_inbox_witness_without_attached_mtd() {
    let (_, module) = target("declare actor Foo receives int;
        define actor Foo as
            receive(ActorMail<int> m): void { return; }
        end").unwrap();
    assert!(module.actors[0].inbox_witnessed);
    assert!(module.mailbox_ingresses.is_empty());
    assert!(!module.actors[0].defined);
}

#[test]
fn sealed_actor_receive_rejects_direct_nominal_and_super_dispatch() {
    for method in [
        "receive(m);",
        "Foo.receive(m);",
        "super.receive(m);",
    ] {
        let source = format!(
            "declare actor Foo receives int;
             mtd (Foo self) receive(ActorMail<int> m): void {{}}
             mtd (Foo self) retry(): void {{ {method} }}"
        );
        assert_eq!(diagnostic(&source), "E_ISOLATION", "{source}");
    }
}

#[test]
fn attached_mtd_fulfills_a_concrete_actor_header_without_inline_receive() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo receives int as end
        mtd (Foo self) receive(ActorMail<int> message): void {}";
    let (_, linked) = target(source).unwrap();
    assert_eq!(linked.mailbox_ingresses.len(), 1);
    assert!(linked.actors[0].inbox_witnessed);
    assert!(!linked.actors[0].defined);
    assert!(!linked.actors[0].emits_witnessed);
}

#[test]
fn declared_and_concrete_receives_cannot_disagree_even_with_valid_declared_mtd() {
    let source = "use namespace workers;
        declare actor Foo receives int;
        define actor Foo receives bool as end
        mtd (Foo self) receive(ActorMail<int> message): void {}";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn attached_receiver_must_match_concrete_inbox_even_when_declaration_omits_it() {
    let source = "use namespace workers;
        declare actor Foo;
        define actor Foo receives int as end
        mtd (Foo self) receive(ActorMail<bool> message): void {}";
    assert_eq!(diagnostic(source), "E_SIGNATURE");
}

#[test]
fn attached_receive_witness_still_does_not_authorize_pipe_emission_or_spawn() {
    let source = "declare actor Foo receives int emits bool;
        mtd (Foo self) receive(ActorMail<int> message): void {}";
    let (_, linked) = target(source).unwrap();
    assert!(linked.actors[0].inbox_witnessed);
    assert!(!linked.actors[0].emits_witnessed);
    assert_eq!(linked.require_spawnable("Foo", Pos::default()).unwrap_err().code, "E_UNLINKED");
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_UNSUPPORTED");
}
