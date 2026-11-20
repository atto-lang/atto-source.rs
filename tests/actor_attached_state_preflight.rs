//! Attached actor state must be a nominal, private schema and must never
//! degrade into ordinary actor fields or guest code before runtime proof.
use atto_compiler::{compile, lexer::lex, target_syntax::parse_target_declarations};

const ATTACHED: &str = r#"
    define actor Counter as end
    define state on actor Counter as {
        let int count;
    }
"#;

fn fail_source(source: &str, code: &str) {
    let d = compile(source).expect_err("invalid/unsupported actor state emitted guest bytes");
    assert_eq!(d.code, code, "{source}: {d}");
}

#[test]
fn actor_state_attached_schema_is_retained_but_guest_emission_fails_closed() {
    let parsed = parse_target_declarations(&lex(ATTACHED).unwrap()).unwrap();
    assert_eq!(parsed.definitions.len(), 1);
    assert_eq!(parsed.actor_state_attachments.len(), 1);
    assert!(parsed.state_owners.is_empty(), "actor attachment is not class state");
    let state = &parsed.actor_state_attachments[0];
    assert_eq!(state.owner, "Counter");
    assert_eq!(state.fields.len(), 1);
    assert_eq!(state.fields[0].name, "count");
    assert_eq!(state.fields[0].kind, "int");
    assert!(state.fields[0].writable);
    assert!(!state.fields[0].initialized);
    fail_source(ATTACHED, "E_UNSUPPORTED");
}

#[test]
fn schema_order_and_literal_scalar_initializers_are_admitted_to_parser_only() {
    let sources = [
        r#"define state on actor Counter as {
               let int count: 0; val bool ready: true;
           } define actor Counter as end"#,
        r#"declare actor Counter;
            define state on actor Counter as {
               let int count = -9223372036854775808;
               val bool ready = false;
            }"#,
    ];
    for source in sources {
        let h = parse_target_declarations(&lex(source).unwrap()).unwrap();
        assert_eq!(h.actor_state_attachments.len(), 1);
        let fields = &h.actor_state_attachments[0].fields;
        assert_eq!(fields.len(), 2);
        assert!(fields.iter().all(|f| f.initialized));
        assert!(fields[0].writable);
        assert!(!fields[1].writable);
        assert_eq!(fields[1].kind, "bool");
        fail_source(source, "E_UNSUPPORTED");
    }
}

#[test]
fn owner_resolution_and_exactly_one_state_schema_are_checked_before_codegen() {
    for (source, code) in [
        ("define state on actor Missing as { let int count; }", "E_UNBOUND"),
        ("define class Counter as end define state on actor Counter as { let int count; }", "E_ACTOR"),
        ("declare struct Counter; define state on actor Counter as { let int count; }", "E_ACTOR"),
        ("define actor Counter as end define state on actor Counter as { let int a; } define state on actor Counter as { let int b; }", "E_DUPLICATE"),
        ("define actor Counter as let int count = 0; end define state on actor Counter as { let int count; }", "E_DUPLICATE"),
        ("define actor Counter as pub count(): int { return 0; } end define state on actor Counter as { let int count; }", "E_DUPLICATE"),
    ] {
        fail_source(source, code);
    }
}

#[test]
fn malformed_scalar_attachments_fail_without_artifacts() {
    for (source, code) in [
        ("define actor C as end define state on actor C as {}", "E_STATE"),
        ("define actor C as end define state on actor C as { int count; }", "E_SYNTAX"),
        ("define actor C as end define state on actor C as { let int count; val bool count; }", "E_DUPLICATE"),
        ("define actor C as end define state on actor C as { let string count; }", "E_UNSUPPORTED_TYPE"),
        ("define actor C as end define state on actor C as { let int count: true; }", "E_SYNTAX"),
        ("define actor C as end define state on actor C as { val bool ready: 3; }", "E_SYNTAX"),
        ("define actor C as end define state on actor C as { let int count: 9223372036854775808; }", "E_SYNTAX"),
        ("define actor C as end define state on actor C as { let int count: 0; ", "E_EOF"),
        ("define actor C as end define state on actor C as { val bool ready; }", "E_STATE"),
        ("define actor C as end define state on actor C as { let int count: 0 }", "E_SYNTAX"),
    ] {
        fail_source(source, code);
    }
}

#[test]
fn the_requested_counter_sample_is_not_silently_compiled_before_guards_and_startup_exist() {
    let source = r#"
        define actor Counter as
            state { let int count; }
            on_start(): void using state.write {
                self.handle_state(critical-write |s| -> { s.count = 0; });
            }
            receive(ActorMail<int> mail): void using state.read, state.write {
                self.handle_state(critical-write |s| -> { s.count += mail.value; });
                val observed = self.handle_state(critical-read |s| -> s.count);
                stdio.println(observed);
            }
        end
        pub fnc main(): int {
            val counter = spawn Counter();
            counter.send(1);
            return 0;
        }
    "#;
    assert!(compile(source).is_err(),
        "full actor startup + lease lowering may be enabled only with compiled guest conformance");
}

#[test]
fn exact_portable_counter_sources_stay_fail_closed_until_real_actor_guest_lowering() {
    // Both files are SHA-identical Git blobs to the positive fixtures in
    // atto-platform-conformance PR #113. This is a *deliberate* unsupported
    // gate, not a claim that either Counter has executed in Wasm.
    let fixtures = [
        ("inline", include_str!("fixtures/actor_attached_state/inline_counter.atto")),
        ("attached", include_str!("fixtures/actor_attached_state/attached_counter.atto")),
    ];
    for (name, source) in fixtures {
        let d = compile(source).expect_err(
            "Counter must not emit guest Wasm until startup, effects, mailbox and guarded actor state are fully checked"
        );
        assert!(d.pos.line > 0 && d.pos.col > 0, "{name}: {d}");
    }
}
