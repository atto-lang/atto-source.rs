//! Runnable Rust tests for the exact source later shown in the playground.
//! These prove target-parser acceptance and negative compiler denial only:
//! no forward-declared actor receiver guest executable is claimed.
use atto_compiler::{compile, lexer::lex};
use atto_compiler::target_syntax::{parse_target_declarations, ReceiverStateWrite};

const PREVIEW: &str = include_str!("playground_previews/actor-named-state-permissions.atto");

#[test]
fn verified_playground_example_is_a_real_target_parser_fixture() {
    let headers = parse_target_declarations(&lex(PREVIEW).unwrap()).unwrap();
    assert_eq!(headers.actors, vec!["Meter".to_string()]);
    assert_eq!(headers.receivers.len(), 5);
    assert_eq!(headers.receivers.iter().map(|x| x.binding.as_str()).collect::<Vec<_>>(),
        ["this", "self", "this", "node", "this"]);
    assert_eq!(headers.receivers[1].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into()])));
    assert_eq!(headers.receivers[2].state_write,
        headers.receivers[1].state_write,
        "self and this must have the same typed write-permission ceiling");
    assert_eq!(headers.receivers[3].state_write,
        Some(ReceiverStateWrite::Fields(vec!["counter".into(), "xyz".into()])));
    assert!(PREVIEW.starts_with("// SYNTAX PREVIEW — NOT browser-lowered"));
    assert_eq!(compile(PREVIEW).unwrap_err().code, "E_UNSUPPORTED",
        "the preview must not compile into a guest until real actor semantics exist");
}

#[test]
fn lexical_receiver_alpha_renaming_is_stable_under_many_identifiers() {
    for binding in ["self", "this", "node", "m", "receiver", "meter42", "_mine"] {
        let src = format!(
            "declare actor Meter; mtd (Meter mut {binding}.state.{{counter, xyz}}) update(): void {{ \
             {binding}.handle_state(critical-write |s| -> {{ s.counter += 1; s.xyz = 3; }}); }}"
        );
        let h = parse_target_declarations(&lex(&src).unwrap()).unwrap();
        assert_eq!(h.receivers[0].binding, binding);
        assert_eq!(h.receivers[0].state_write,
            Some(ReceiverStateWrite::Fields(vec!["counter".into(), "xyz".into()])));
        assert_eq!(compile(&src).unwrap_err().code, "E_UNSUPPORTED");
    }
}

#[test]
fn lexical_receiver_rejects_reserved_keyword_aliases() {
    for name in ["actor", "return", "await", "mut", "switch", "fnc", "if",
                 "declare", "mtd", "use", "select", "spawn", "const"] {
        let src = format!("declare actor Meter; mtd (Meter mut {name}.state.counter) f(): void {{}}");
        let error = parse_target_declarations(&lex(&src).unwrap()).unwrap_err();
        assert_eq!(error.code, "E_SYNTAX", "reserved receiver {name} unexpectedly admitted: {error}");
    }
}

#[test]
fn user_state_protection_applies_to_renamed_receivers_even_inside_calls() {
    for (body, code) in [
        ("this.send(this.state.counter);", "E_SEALED"),
        ("this.send(this);", "E_ESCAPE"),
        ("this.send(this.not_declared());", "E_UNBOUND"),
        ("this.state.counter = 1;", "E_SEALED"),
        ("this.handle_state(critical-write |s| -> { s.xyz = 2; });", "E_MUTABILITY"),
        ("this.handle_state(critical-read |s| -> { ++s.counter; });", "E_READONLY"),
        ("this.handle_state(critical-write |s| -> { s.xyz.inner = 1; });", "E_UNSUPPORTED"),
    ] {
        let src = format!(
            "declare actor Meter; mtd (Meter mut this.state.counter) f(): void {{ {body} }}"
        );
        let error = parse_target_declarations(&lex(&src).unwrap()).unwrap_err();
        assert_eq!(error.code, code, "wrong diagnostic for {body}: {error}");
    }
}

#[test]
fn method_identity_does_not_depend_on_named_receiver_binding() {
    let src = "declare actor Meter; \
        mtd (Meter mut self.state.counter) update(): void {} \
        mtd (Meter mut this.state.counter) update(): void {}";
    let error = parse_target_declarations(&lex(src).unwrap()).unwrap_err();
    assert_eq!(error.code, "E_DUPLICATE");
}
