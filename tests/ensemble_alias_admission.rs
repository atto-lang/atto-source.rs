use atto_compiler::{compile, compile_js, compile_with_imports, compile_with_path};
use std::path::Path;

const CANONICAL: &str = include_str!("fixtures/atto-ensemble-v1/canonical-unlowered-try-publish.atto");
const CODE: &str = "E_ACTOR_PUBSUB_UNSUPPORTED";

#[test]
fn canonical_alias_is_rejected_before_wasm_and_js_emission() {
    assert_eq!(compile(CANONICAL).unwrap_err().code, CODE);
    assert_eq!(compile_js(CANONICAL).unwrap_err().code, CODE);
    assert_eq!(compile_with_path(CANONICAL, Path::new("main.atto")).unwrap_err().code, CODE);
}

#[test]
fn cli_linking_path_rejects_before_loader() {
    let mut calls = 0;
    let result = compile_with_imports(CANONICAL, Path::new("main.atto"), |_| {
        calls += 1;
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unexpected load"))
    });
    assert_eq!(result.unwrap_err().code, CODE);
    assert_eq!(calls, 0);
}

#[test]
fn comments_strings_and_ordinary_identifiers_remain_inert() {
    let src = r#"pub fnc main(): int {
        // imported_topic.try_publish(share! event)?
        /* imported_topic.try_publish(share! event)? */
        return 42;
    }"#;
    assert!(compile(src).is_ok());
    let src = r#"pub fnc main(): int {
        let string text = "imported_topic.try_publish(share! event)?";
        return 42;
    }"#;
    assert_ne!(compile(src).err().map(|e| e.code), Some(CODE));
    let src = "pub fnc main(): int { let int try_publish = 42; return try_publish; }";
    assert_ne!(compile(src).err().map(|e| e.code), Some(CODE));
}

#[test]
fn lexical_trivia_and_alias_spelling_do_not_bypass_gate() {
    for body in [
        "local_topic.try_publish(share! event)?",
        "local_topic /* trivia */ . try_publish (share! event)?",
        "local_topic.try_publish(share !event)?",
    ] {
        let src = format!("pub fnc main(): void {{ val receipt = {body}; return; }}");
        assert_eq!(compile(&src).unwrap_err().code, CODE, "{body}");
    }
}
