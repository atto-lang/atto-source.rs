//! Regression gate for merging the 2026-11 typed actor ingress compiler
//! into the later persistence-aware default-branch parser. Parse assertions
//! deliberately do not certify mailbox scheduling or TenantVM confinement.
use atto_compiler::ast::{ActorKind, ActorPersistenceMode};
use atto_compiler::{check, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("lex"))
        .parse_program().expect("parse")
}
fn actor(decl: &str) -> atto_compiler::ast::ActorDecl {
    let source=format!("{decl}\npub fnc main(): int {{ return 0; }}");
    let mut program=parse(&source);
    assert_eq!(program.actors.len(),1);
    program.actors.remove(0)
}

#[test]
fn actor_kind_and_typed_payload_remain_distinct_after_mainline_merge() {
    let ordinary=actor("define actor Counter as receive(ActorMail<int> mail): void { return; } end");
    assert_eq!(ordinary.kind,ActorKind::Default);
    assert!(ordinary.receive.unwrap().envelope);

    let serial=actor("define serial actor Counter as receive(msg: int): void { return; } end");
    assert_eq!(serial.kind,ActorKind::Serial);
    let recv=serial.receive.unwrap();
    assert!(!recv.envelope);
    assert_eq!(recv.mail,"msg");

    let legacy=actor("serial actor Counter receive(ActorMail<bool> mail): void { return; } end");
    assert_eq!(legacy.kind,ActorKind::Serial);
    assert!(legacy.receive.unwrap().envelope);
}

#[test]
fn newer_actor_persistence_qualifiers_survive_typed_receiver_merge() {
    let persisted=actor(
        "define persistent actor PersistentCounter as receive(ActorMail<int> mail): void { return; } end");
    assert_eq!(persisted.persistence,ActorPersistenceMode::Persistent);
    assert_eq!(persisted.kind,ActorKind::Default);

    let durable=actor(
        "define durable actor DurableCounter as receive(ActorMail<int> mail): void { return; } end");
    assert_eq!(durable.persistence,ActorPersistenceMode::Durable);
    assert_eq!(durable.kind,ActorKind::Default);
}

#[test]
fn unproved_shared_or_isolated_actors_cannot_lower_to_browser_wasm() {
    for source in [
        "define isoactor Guest as receive(msg: int): void { return; } end",
        "define shared actor Guest as receive(msg: int): void { return; } end",
        "define untrusted actor Guest as receive(msg: int): void { return; } end",
    ] {
        let p=parse(&format!("{source}\npub fnc main(): int {{ return 0; }}"));
        let e=check::check(&p).expect_err("unproved actor isolation must fail closed");
        assert_eq!(e.code,"E_UNSUPPORTED");
    }
}

#[test]
fn direct_bool_ingress_needs_tagged_wire_transport() {
    let src="define serial actor B as receive(msg: bool): void { return; } end\npub fnc main(): int { return 0; }";
    let e=check::check(&parse(src)).expect_err("direct bool is not yet admitted");
    assert_eq!(e.code,"E_UNSUPPORTED_TYPE");
}

#[test]
fn actor_without_receive_is_rejected_except_at_pure_syntax_level() {
    let src="define actor Worker as end\npub fnc main(): int { return 0; }";
    let p=parse(src);
    assert!(p.actors[0].receive.is_none());
    assert_eq!(check::check(&p).unwrap_err().code,"E_TYPE");
}
