//! Source-admission parity with the .NET NativeAOT and JS compiler HTTP actor gates.
//! These are real Rust lexer/parser and Cranelift object-emitter tests, not HTTP
//! guest execution or a claim that Rust can compile Spin.
use oreslang_source_rs::{compile, compile_with_effect_ir, frontend};

const UNSUPPORTED: &str = "HTTP_ACTOR_BACKEND_UNSUPPORTED";

fn rejected(source: &str) {
    let parsed = frontend::parse(source).expect_err("unsupported HTTP actor parsed");
    assert_eq!(parsed.class, "http_actor_backend_unsupported", "{source}");
    assert!(parsed.message.starts_with(UNSUPPORTED), "{parsed}");
    assert!(parsed.offset < source.len(), "{parsed}");
    let compiled = compile(source).expect_err("unsupported HTTP actor emitted object");
    assert!(compiled.contains(UNSUPPORTED), "{compiled}");
    let paired = compile_with_effect_ir(source).expect_err("unverified HTTP actor emitted effects");
    assert!(paired.contains(UNSUPPORTED), "{paired}");
}

#[test]
fn shared_http_actor_3x4_atto_fixture_fails_closed() {
    rejected(include_str!("fixtures/http-actor-unsupported.atto"));
}

#[test]
fn serial_1x1_public_mixed_modifiers_and_isoactors_fail_closed() {
    for source in [
        "define serial http actor Endpoint as end",
        "define http actor Endpoint as int requestConcurrency = 1; int messageConcurrency = 1; end",
        "define http actor Endpoint as int requestConcurrency = 3; int messageConcurrency = 4; end",
        "define pub shared http actor Endpoint as end",
        "pub define untrusted http actor Endpoint as end",
        "pub define http isoactor Endpoint as end",
        "define private tcp http stream actor Endpoint as end",
    ] {
        rejected(source);
    }
}

#[test]
fn comments_do_not_grant_or_remove_http_actor_admission() {
    let source = concat!(
        "/* define http actor Hidden as end */\n",
        "// define serial http actor Hidden as end\n",
        "fnc http(): int { return 42; }\n",
        "pub fnc main(): int { return http(); }"
    );
    assert!(frontend::parse(source).is_ok());
    assert!(!compile(source)
        .expect("valid http identifier object")
        .is_empty());
    // The unsupported declaration remains visible through adjacent comments.
    rejected("/* define http actor Hidden as end */\n define http actor Endpoint as end");
}

#[test]
fn unrelated_actor_kind_not_mislabeled_as_source_http() {
    let err = frontend::parse("define actor Foo as end").expect_err("actors not implemented");
    assert_ne!(err.class, "http_actor_backend_unsupported");
}

#[test]
fn unicode_line_separators_cannot_hide_http_actor_after_comment() {
    for separator in ["\r", "\n", "\u{0085}", "\u{2028}", "\u{2029}"] {
        let source = format!("// inert actor words{separator}define http actor Endpoint as end");
        rejected(&source);
        let valid = format!(
            "// define http actor Inert as end{separator}\
             fnc http(): int {{ return 42; }}\
             pub fnc main(): int {{ return http(); }}"
        );
        assert!(frontend::parse(&valid).is_ok(), "separator: {separator:?}");
        assert!(!compile(&valid).expect("valid http identifier source").is_empty());
    }
}
