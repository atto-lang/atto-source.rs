//! Nominal reply-mode typed IR tests: no compiled guest reply support.
use atto_compiler::actor_reply_contract::{check_unary, resolve_message_mode};
use atto_compiler::actor_reply_syntax::inspect;
use atto_compiler::ast::ActorResponseMode;
use atto_compiler::lexer::lex;
use atto_compiler::compile;

fn check(src: &str) -> Result<Vec<atto_compiler::ast::ActorMessageContract>,
    atto_compiler::Diagnostic> {
    let tokens = lex(src)?;
    let signatures = inspect(&tokens)?;
    check_unary(&tokens, &signatures)
}
fn err(src: &str) -> &'static str { check(src).unwrap_err().code }
fn handler(request: &str, response: &str) -> String {
    format!("define unary actor Service receives {request} sends {response} as
        receive(ActorMail<{request}> mail): {response} {{
            return new {response}();
        }} end")
}
#[test]
fn direct_unary_interface_is_linked_to_typed_reply_ir() {
    let src = format!(
        "define class GetUser implements UnaryRequest<User> as end
         define class User as end {}", handler("GetUser", "User")
    );
    let contracts = check(&src).unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].actor, "Service");
    assert_eq!(contracts[0].message, "GetUser");
    assert_eq!(contracts[0].response, ActorResponseMode::Unary("User".into()));
    assert_eq!(compile(&src).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn transitive_interface_inheritance_resolves_unary_payload() {
    let src = format!(
        "define interface Fetch extends UnaryRequest<User> as end
         define class GetUser implements Fetch as end {}",
        handler("GetUser", "User")
    );
    let m = check(&src).unwrap();
    assert_eq!(m[0].response, ActorResponseMode::Unary("User".into()));
}
#[test]
fn one_way_is_not_mistaken_for_unary() {
    let src = format!(
        "define class GetUser implements OneWayMessage as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_MODE");
    assert_eq!(compile(&src).unwrap_err().code, "E_REPLY_MODE");
}
#[test]
fn stream_mode_is_not_mistaken_for_unary() {
    let src = format!(
        "define class GetUser implements StreamRequest<Event> as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_MODE");
}
#[test]
fn incorrect_handler_response_type_is_diagnostic() {
    let src = format!(
        "define class GetUser implements UnaryRequest<User> as end {}",
        handler("GetUser", "Other")
    );
    assert_eq!(err(&src), "E_REPLY_TYPE");
}
#[test]
fn overlapping_direct_modes_rejected() {
    let src = format!(
        "define class GetUser implements UnaryRequest<User>, StreamRequest<Event> as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_MODE");
}
#[test]
fn inherited_conflicting_reply_modes_rejected() {
    let src = format!(
        "define interface A extends UnaryRequest<User> as end
         define interface B extends StreamRequest<Event> as end
         define class GetUser implements A, B as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_MODE");
}
#[test]
fn bare_unary_request_contract_is_not_valid() {
    let src = format!(
        "define class GetUser implements UnaryRequest as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_TYPE");
}
#[test]
fn erased_any_reply_payload_is_rejected() {
    let src = format!(
        "define class GetUser implements UnaryRequest<Any> as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_TYPE");
}
#[test]
fn declared_class_without_response_mode_is_rejected() {
    let src = format!(
        "define class GetUser implements Named as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_MODE");
}
#[test]
fn unknown_external_message_never_proves_a_typed_contract() {
    let src = handler("ExternalMessage", "Response");
    assert!(check(&src).unwrap().is_empty());
    assert_eq!(compile(&src).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn unrelated_class_inherits_no_implicit_response_mode() {
    let t = lex("define class User as end define class Other extends User as end").unwrap();
    assert_eq!(resolve_message_mode(&t, "Other").unwrap(), None);
}
#[test]
fn inherited_stream_mode_is_resolved_without_granting_unary_reply() {
    let t = lex("define interface I extends StreamRequest<Event> as end
        define class Sub implements I as end").unwrap();
    assert_eq!(resolve_message_mode(&t, "Sub").unwrap(),
        Some(ActorResponseMode::Stream("Event".into())));
}
#[test]
fn strings_and_comments_do_not_create_fake_reply_interfaces() {
    let t = lex("pub fnc main(): int { // define class Fake implements UnaryRequest<Foo> as end
      val text: string = \"define class Fake implements UnaryRequest<Foo> as end\";
      return 0; }").unwrap();
    assert_eq!(resolve_message_mode(&t, "Fake").unwrap(), None);
}
#[test]
fn cycles_in_nominal_reply_graph_are_rejected() {
    let src = format!(
        "define interface A extends B as end
         define interface B extends A as end
         define class GetUser implements A as end {}",
        handler("GetUser", "User")
    );
    assert_eq!(err(&src), "E_REPLY_MODE");
}
