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
