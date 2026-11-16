//! Stage-1 unary actor signature preflight. No executable actor reply guest ABI.
use atto_compiler::actor_reply_syntax::{inspect, uses_reply_intrinsic};
use atto_compiler::lexer::lex;
use atto_compiler::compile;

fn signature(src: &str) -> Result<Vec<atto_compiler::actor_reply_syntax::UnarySignature>, atto_compiler::Diagnostic> {
    inspect(&lex(src).unwrap())
}
fn diagnostic(src: &str) -> &'static str {
    match signature(src) {
        Ok(_) => panic!("expected signature diagnostic for {src}"),
        Err(e) => e.code,
    }
}
const OK_HTTP: &str = r#"
    define unary http actor Foo
        receives HTTPRequest
        sends HTTPResponse
    as
        susp receive(ActorMail<HTTPRequest> m): HTTPResponse {
            const result = await process(m.value);
            return new HTTPResponse(200, result);
        }
    end
"#;

#[test]
fn unary_http_signature_recognized_without_claiming_executable_reply_abi() {
    let defs = signature(OK_HTTP).unwrap();
    assert_eq!(defs.len(), 1);
    let s = &defs[0];
    assert_eq!(s.actor, "Foo");
    assert_eq!(s.request, "HTTPRequest");
    assert_eq!(s.response, "HTTPResponse");
    assert!(s.http && s.suspends);
    assert_eq!(compile(OK_HTTP).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn plain_unary_actor_accepts_nominal_signature_as_metadata_only() {
    let src = "define unary actor Foo receives GetUser sends User as
        receive(ActorMail<GetUser> m): User { return new User(); } end";
    let defs = signature(src).unwrap();
    assert!(!defs[0].http && !defs[0].suspends);
    assert_eq!(defs[0].response, "User");
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn emitted_pipeline_type_does_not_become_reply_type() {
    let src = "define unary actor Foo receives Read emits Event sends Response as
       receive(ActorMail<Read> m): Response { emit(new Event()); return new Response(); } end";
    let defs = signature(src).unwrap();
    assert_eq!(defs[0].request, "Read");
    assert_eq!(defs[0].response, "Response");
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn mismatch_between_receives_clause_and_mail_envelope_is_a_hard_error() {
    let src = "define unary http actor Foo receives HTTPRequest as
        receive(ActorMail<Other> m): HTTPResponse { return new HTTPResponse(); } end";
    assert_eq!(diagnostic(src), "E_REPLY_TYPE");
    assert_eq!(compile(src).unwrap_err().code, "E_REPLY_TYPE");
}

#[test]
fn missing_or_void_unary_response_is_rejected() {
    for src in [
        "define unary actor Foo receives A as receive(ActorMail<A> m) { return 1; } end",
        "define unary actor Foo receives A as receive(ActorMail<A> m): void { return; } end",
    ] {
        assert!(matches!(diagnostic(src), "E_REPLY_SIGNATURE" | "E_REPLY_TYPE"), "{src}");
    }
}

#[test]
fn erased_mailbox_payload_does_not_grant_reply_authority() {
    let src = "define unary actor Foo receives A as
        receive(ActorMail<any> m): B { return new B(); } end";
    assert_eq!(diagnostic(src), "E_REPLY_TYPE");
}

#[test]
fn declared_single_type_sends_must_cover_reply_type() {
    let src = "define unary actor Foo receives Query sends Event as
        receive(ActorMail<Query> m): Reply { return new Reply(); } end";
    assert_eq!(diagnostic(src), "E_REPLY_TYPE");
}

#[test]
fn duplicate_or_out_of_order_actor_headers_are_rejected() {
    for src in [
        "define unary actor Foo receives Query receives Query as receive(ActorMail<Query> m): Reply { return new Reply(); } end",
        "define unary actor Foo emits Event receives Query as receive(ActorMail<Query> m): Reply { return new Reply(); } end",
        "define unary actor Foo receives Query sends Reply sends Reply as receive(ActorMail<Query> m): Reply { return new Reply(); } end",
    ] {
        assert_eq!(diagnostic(src), "E_REPLY_SIGNATURE", "{src}");
    }
}

#[test]
fn explicit_reply_and_implicit_unary_return_cannot_be_mixed() {
    let src = "define unary actor Foo receives Query as
       receive(ActorMail<Query> m): Reply {
           m.respond_with(new Reply());
           return new Reply();
       } end";
    assert_eq!(diagnostic(src), "E_REPLY_LINEAR");
    assert_eq!(compile(src).unwrap_err().code, "E_REPLY_LINEAR");
    let alias = src.replace("respond_with", "send_back");
    assert_eq!(diagnostic(&alias), "E_REPLY_LINEAR");
}

#[test]
fn unary_handler_without_return_is_missing_reply() {
    let src = "define unary actor Foo receives Query as
      receive(ActorMail<Query> m): Reply { const x = 42; } end";
    assert_eq!(diagnostic(src), "E_REPLY_MISSING");
}

#[test]
fn ordinary_actor_reply_sugar_is_explicitly_fail_closed() {
    let src = "define actor Foo receives Query as
       receive(ActorMail<Query> m): void { m.respond_with(42); } end";
    assert!(uses_reply_intrinsic(&lex(src).unwrap()));
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn non_reply_ordinary_actor_is_unchanged() {
    let src = "define actor Foo receives int as
        receive(ActorMail<int> m): void { return; }
        end
        pub fnc main(): int { return 0; }";
    assert!(signature(src).unwrap().is_empty());
    assert!(!uses_reply_intrinsic(&lex(src).unwrap()));
    assert!(compile(src).is_ok());
}

#[test]
fn comments_and_strings_do_not_trigger_unary_or_reply_sugar() {
    let src = r#"pub fnc main(): int {
        // define unary actor Foo ...
        val x: string = "define unary actor Foo m.respond_with()";
        return 0;
    }"#;
    assert!(signature(src).unwrap().is_empty());
    assert!(!uses_reply_intrinsic(&lex(src).unwrap()));
    assert!(compile(src).is_ok());
}

#[test]
fn malformed_unary_brace_termination_is_rejected() {
    let src = "define unary actor Foo receives A as receive(ActorMail<A> m): B { return new B();";
    assert_eq!(diagnostic(src), "E_EOF");
}

#[test]
fn more_than_one_unary_signature_is_collected() {
    let src = "define unary actor A receives Req as receive(ActorMail<Req> m): Out { return new Out(); } end
               define unary actor B receives Get as receive(ActorMail<Get> m): Value { return new Value(); } end";
    let sigs = signature(src).unwrap();
    assert_eq!(sigs.len(), 2);
    assert_eq!(sigs[1].actor, "B");
}
