//! Ordinary actor reply preflight. Guest request/reply ABI stays unsupported.
use atto_compiler::actor_reply_contract::check_explicit_replies;
use atto_compiler::ast::ActorResponseMode;
use atto_compiler::lexer::lex;
use atto_compiler::compile;

fn check(s: &str) -> Result<Vec<atto_compiler::ast::ActorMessageContract>,
    atto_compiler::Diagnostic> {
    check_explicit_replies(&lex(s)?)
}
fn src(marker: &str, body: &str) -> String {
    format!("define class GetUser implements {marker} as end
        define actor Directory receives GetUser sends User as
            receive(ActorMail<GetUser> m): void {{
                {body}
            }}
        end")
}
#[test]
fn explicit_unary_reply_has_nominal_typed_ir() {
    let s=src("UnaryRequest<User>", "m.respond_with(new User());");
    let h=check(&s).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].actor, "Directory");
    assert_eq!(h[0].message, "GetUser");
    assert_eq!(h[0].response, ActorResponseMode::Unary("User".into()));
    assert_eq!(compile(&s).unwrap_err().code,"E_UNSUPPORTED");
}
#[test]
fn unary_reply_rejects_wrong_constructor_type() {
    let s=src("UnaryRequest<User>", "m.respond_with(new Other());");
    assert_eq!(check(&s).unwrap_err().code, "E_REPLY_TYPE");
    assert_eq!(compile(&s).unwrap_err().code, "E_REPLY_TYPE");
}
#[test]
fn unary_reply_not_authorized_for_oneway_message() {
    let s=src("OneWayMessage","m.respond_with(new User());");
    assert_eq!(check(&s).unwrap_err().code,"E_REPLY_MODE");
    assert_eq!(compile(&s).unwrap_err().code,"E_REPLY_MODE");
}
#[test]
fn stream_message_cannot_use_unary_response() {
    let s=src("StreamRequest<User>","m.respond_with(new User());");
    assert_eq!(check(&s).unwrap_err().code,"E_REPLY_MODE");
}
#[test]
fn unary_request_cannot_obtain_reply_stream() {
    let s=src("UnaryRequest<User>","m.reply_stream();");
    assert_eq!(check(&s).unwrap_err().code,"E_REPLY_MODE");
}
#[test]
fn streaming_request_can_get_typed_stream_capability_metadata_only() {
    let s=src("StreamRequest<User>","m.reply_stream();");
    let out=check(&s).unwrap();
    assert_eq!(out[0].response,ActorResponseMode::Stream("User".into()));
    assert_eq!(compile(&s).unwrap_err().code,"E_UNSUPPORTED");
}
#[test]
fn noarg_unary_reply_requires_explicit_unit_type() {
    let s=src("UnaryRequest<User>","m.respond_with();");
    assert_eq!(check(&s).unwrap_err().code,"E_REPLY_TYPE");
}
#[test]
fn multiple_possible_reply_sites_require_real_path_analysis() {
    let s=src("UnaryRequest<User>",
        "m.respond_with(new User()); m.respond_with(new User());");
    assert_eq!(check(&s).unwrap_err().code,"E_UNSUPPORTED");
}
#[test]
fn absent_nominal_contract_must_not_invent_response_authority() {
    let s="define actor Directory receives GetUser as
        receive(ActorMail<GetUser> m): void {
            m.respond_with(new User());
        } end";
    assert!(check(s).unwrap().is_empty());
    assert_eq!(compile(s).unwrap_err().code,"E_UNSUPPORTED");
}
#[test]
fn ordinary_actor_without_explicit_reply_is_not_affected() {
    let s="define actor Directory receives int as
        receive(ActorMail<int> m): void { return; } end
        pub fnc main(): int { return 0; }";
    assert!(check(s).unwrap().is_empty());
    assert!(compile(s).is_ok());
}
