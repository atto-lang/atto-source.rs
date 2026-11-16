//! Production AST admission: unary/http/susp receive must be represented
//! without granting an executable guest reply ABI.
use atto_compiler::ast::{ExprKind, Stmt, Type};
use atto_compiler::lexer::lex;
use atto_compiler::parser::Parser;
use atto_compiler::compile;

fn parse(s: &str) -> Result<atto_compiler::ast::Program, atto_compiler::Diagnostic> {
    Parser::new(lex(s)?).parse_program()
}

#[test]
fn unary_http_actor_preserves_modifiers_return_type_and_suspension_in_ast() {
    let source = "define unary http actor Foo receives HTTPRequest sends HTTPResponse as
      susp receive(ActorMail<HTTPRequest> m): HTTPResponse {
        return new HTTPResponse();
      }
    end";
    let p = parse(source).unwrap();
    let a = &p.actors[0];
    assert!(a.is_unary);
    assert!(a.is_http);
    let r = a.receive.as_ref().unwrap();
    assert!(r.suspends);
    assert_ne!(r.ret, Type::Void);
    assert_eq!(r.mail, "m");
    assert!(matches!(&r.body[0], Stmt::Return(Some(e), _)
        if matches!(&e.kind, ExprKind::New(name, _) if name == "HTTPResponse")));
    assert_eq!(compile(source).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn unary_actor_without_http_is_not_forced_serial_or_networked() {
    let p = parse("define unary actor Foo receives Query as
      receive(ActorMail<Query> m): Response { return new Response(); } end").unwrap();
    assert!(p.actors[0].is_unary);
    assert!(!p.actors[0].is_http);
    assert!(!p.actors[0].receive.as_ref().unwrap().suspends);
}
#[test]
fn ordinary_http_actor_is_not_automatically_unary() {
    let p = parse("define http actor Foo receives int as
      receive(ActorMail<int> m): void { return; } end").unwrap();
    assert!(p.actors[0].is_http);
    assert!(!p.actors[0].is_unary);
}
#[test]
fn ordinary_actor_can_parse_suspended_void_receive_but_not_emit_guest_abi() {
    let s = "define actor Foo receives int as
      susp receive(ActorMail<int> m): void { return; } end
      pub fnc main(): int { return 0; }";
    let p = parse(s).unwrap();
    assert!(p.actors[0].receive.as_ref().unwrap().suspends);
    assert_eq!(compile(s).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn ordinary_actor_must_keep_void_ingress() {
    let s = "define actor Foo receives int as
      receive(ActorMail<int> m): int { return 1; } end";
    assert_eq!(parse(s).unwrap_err().code, "E_TYPE");
}
#[test]
fn unary_actor_requires_nonvoid_ingress() {
    let s = "define unary actor Foo receives int as
      receive(ActorMail<int> m): void { return; } end";
    assert_eq!(parse(s).unwrap_err().code, "E_REPLY_TYPE");
}
#[test]
fn duplicate_susp_and_regular_ingress_rejected() {
    let s = "define actor Foo receives int as
      susp receive(ActorMail<int> a): void { return; }
      receive(ActorMail<int> b): void { return; }
    end";
    assert_eq!(parse(s).unwrap_err().code, "E_DUPLICATE");
}
#[test]
fn ordinary_actor_legacy_ast_remains_unmodified() {
    let s = "define actor Foo receives int as
      receive(ActorMail<int> m): void { return; } end
      pub fnc main(): int { return 0; }";
    let p = parse(s).unwrap();
    assert!(!p.actors[0].is_unary && !p.actors[0].is_http);
    let r = p.actors[0].receive.as_ref().unwrap();
    assert!(!r.suspends && r.ret == Type::Void);
    assert!(compile(s).is_ok());
}
#[test]
fn public_unary_actor_is_not_silently_admitted_as_private() {
    let s = "pub define unary actor Foo receives Query as
      receive(ActorMail<Query> m): Reply { return new Reply(); } end";
    assert_eq!(parse(s).unwrap_err().code, "E_UNSUPPORTED");
}
