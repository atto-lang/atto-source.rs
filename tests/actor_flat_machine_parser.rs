//! Production parser boundary tests for a restricted flat state machine.
//! Not a guest execution test or a complete grammar implementation.
use atto_compiler::{compile, lexer, parser::Parser};

const GOOD: &str = "define actor Foo as
    state machine as
        initial state Idle;
        state Idle as
            on Start() -> Running;
        end
        state Running as
            on Finish() -> Complete;
        end
        final state Complete;
    end
end";

fn parser_code(src: &str, expected: &str) {
    let mut parser = Parser::new(lexer::lex(src).expect("valid lexing"));
    match parser.parse_program() {
        Ok(_) => panic!("unimplemented machine must not be admitted"),
        Err(err) => assert_eq!(err.code, expected, "{err}"),
    }
    assert!(compile(src).is_err(), "no unverified guest artifact");
}

#[test]
fn valid_graph_has_explicit_lowering_gate() {
    parser_code(GOOD, "E_MACHINE_LOWERING_UNSUPPORTED");
}

#[test]
fn malformed_graphs_fail_before_lowering() {
    for (from, to) in [
        ("initial state Idle;", ""),
        ("initial state Idle;", "initial state Idle; initial state Idle;"),
        ("initial state Idle;", "initial state Unknown;"),
        ("Start() -> Running;", "Start() -> Missing;"),
        ("Start() -> Running;", "Start() -> Running; on Start() -> Running;"),
        ("final state Complete;", "final state Complete; final state Complete;"),
        ("final state Complete;", "final state Complete; final state Orphan;"),
    ] {
        parser_code(&GOOD.replace(from, to), "E_MACHINE_GRAPH");
    }
}

#[test]
fn contextual_machine_does_not_reserve_ordinary_field_name() {
    let source = "define actor Foo as state machine: int = 0; end";
    let mut parser = Parser::new(lexer::lex(source).unwrap());
    assert!(parser.parse_program().is_ok());
}

#[test]
fn graph_size_is_bounded() {
    let mut source = String::from("define actor Foo as state machine as initial state Idle; state Idle as ");
    for i in 0..257 {
        source.push_str(&format!("on Event{i}() -> Idle; "));
    }
    source.push_str("end end end");
    parser_code(&source, "E_SOURCE_LIMIT");
}

// A valid first graph must not hide a second declaration or broken actor tail.
#[test]
fn duplicate_machine_and_actor_tail_are_checked() {
    let machine = "state machine as initial state Idle; final state Idle; end";
    parser_code(&format!("define actor Foo as {machine} {machine} end"), "E_MACHINE_GRAPH");
    parser_code(&format!("define actor Foo as pub {machine} end"), "E_SYNTAX");
    let source = format!("define actor Foo as {machine}");
    assert!(Parser::new(lexer::lex(&source).unwrap()).parse_program().is_err());
    assert!(compile(&source).is_err());
}

#[test]
fn node_limit_is_bounded() {
    let mut source = String::from("define actor Foo as state machine as initial state Idle; final state Idle; ");
    for i in 0..256 {
        source.push_str(&format!("final state Node{i}; "));
    }
    source.push_str("end end");
    parser_code(&source, "E_SOURCE_LIMIT");
}

#[test]
fn inspection_retains_resolved_graph_and_separate_storage() {
    let source = GOOD.replace("state machine as", "state { let int count: 0; } state machine as");
    let inspected = Parser::inspect_actor_machines(&source).expect("metadata only");
    assert_eq!(inspected.len(), 1);
    let actor = &inspected[0];
    assert_eq!(actor.actor, "Foo");
    assert_eq!(actor.storage.as_ref().unwrap()[0].name, "count");
    let graph = actor.machine.as_ref().unwrap();
    assert_eq!(graph.nodes[graph.initial].name, "Idle");
    let (event, destination) = &graph.nodes[graph.initial].transitions[0];
    assert_eq!(event, "Start");
    assert_eq!(graph.nodes[*destination].name, "Running");
    let terminal = graph.nodes.iter().find(|n| n.name == "Complete").unwrap();
    assert!(terminal.terminal && terminal.transitions.is_empty());
    parser_code(&source, "E_MACHINE_LOWERING_UNSUPPORTED");
}

#[test]
fn empty_storage_is_retained_without_synthesizing_fields() {
    let source = "define actor Foo as state {} end";
    let inspected = Parser::inspect_actor_machines(source).unwrap();
    assert_eq!(inspected.len(), 1);
    assert!(inspected[0].storage.as_ref().unwrap().is_empty());
    assert!(inspected[0].machine.is_none());
    parser_code(source, "E_UNSUPPORTED");
}

#[test]
fn inspection_rejects_duplicate_or_mixed_storage() {
    for source in [
        "define actor Foo as state {} state {} end",
        "define actor Foo as state {} state machine: int = 0; end",
        "define actor Foo as state { let int n: 0; let int n: 1; } end",
    ] {
        assert!(Parser::inspect_actor_machines(source).is_err());
        assert!(compile(source).is_err());
    }
}

#[test]
fn shared_inspection_vectors_have_canonical_graph_order() {
    let cycle = include_str!("fixtures/actor-machine-inspection-v1/flat-cycle.atto");
    let order = include_str!("fixtures/actor-machine-inspection-v1/flat-order.atto");
    let actors = Parser::inspect_actor_machines(cycle).unwrap();
    let graph = actors[0].machine.as_ref().unwrap();
    assert_eq!(graph.initial, 0);
    assert_eq!(graph.nodes[0].name, "Idle");
    assert_eq!(graph.nodes[0].transitions, vec![("Start".to_string(), 1)]);
    assert_eq!(graph.nodes[1].name, "Running");
    assert_eq!(graph.nodes[1].transitions, vec![("Stop".to_string(), 0)]);
    let actors = Parser::inspect_actor_machines(order).unwrap();
    let graph = actors[0].machine.as_ref().unwrap();
    assert!(actors[0].storage.as_ref().unwrap().is_empty());
    assert_eq!(graph.initial, 1);
    assert_eq!(graph.nodes[0].name, "Alpha");
    assert!(graph.nodes[0].terminal);
    assert!(graph.nodes[0].transitions.is_empty());
    assert_eq!(graph.nodes[1].name, "Zed");
    assert_eq!(graph.nodes[1].transitions, vec![("AlphaEvent".to_string(), 0), ("Zulu".to_string(), 1)]);
    for source in [cycle, order] {
        assert!(compile(source).is_err());
    }
}

#[test]
fn shared_storage_vectors_reject_invalid_initializers() {
    for source in [
        include_str!("fixtures/actor-machine-inspection-v1/wrong-scalar.atto"),
        include_str!("fixtures/actor-machine-inspection-v1/effectful-initializer.atto"),
    ] {
        assert!(Parser::inspect_actor_machines(source).is_err());
        assert!(compile(source).is_err());
    }
}

#[test]
fn graph_table_verifies_slots_and_rejects_forged_metadata() {
    let src = include_str!("fixtures/actor-machine-inspection-v1/flat-order.atto");
    let actors = Parser::inspect_actor_machines(src).unwrap();
    let graph = actors[0].machine.as_ref().unwrap();
    let table = graph.transition_table().unwrap();
    assert_eq!(table.initial(), 1);
    assert_eq!(table.names(), &["Alpha".to_string(), "Zed".to_string()]);
    assert_eq!(table.events(), &["AlphaEvent".to_string(), "Zulu".to_string()]);
    assert_eq!(table.successor(1, 0), Some(Some(0)));
    assert_eq!(table.successor(1, 1), Some(Some(1)));
    assert_eq!(table.successor(0, 0), Some(None));
    assert_eq!(table.successor(2, 0), None);
    assert_eq!(table.successor(1, 2), None);
    let mut bad = graph.clone();
    bad.nodes[1].transitions[0].1 = 9;
    assert!(bad.transition_table().is_err());
    let mut bad = graph.clone();
    bad.nodes[1].transitions.reverse();
    assert!(bad.transition_table().is_err());
    let mut bad = graph.clone();
    bad.nodes[0].transitions.push(("X".into(), 1));
    assert!(bad.transition_table().is_err());
    let mut bad = graph.clone();
    bad.nodes[1].transitions.remove(0);
    assert!(bad.transition_table().is_err());
}
