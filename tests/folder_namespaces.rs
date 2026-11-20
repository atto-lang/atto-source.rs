use std::path::Path;
use atto_compiler::{lexer::{lex, Tok}, namespace, target_syntax};

fn resolve(source: &str, path: &str) -> String {
    let normalized = namespace::normalize(&lex(source).unwrap(), Some(Path::new(path))).unwrap();
    let mut header = Vec::new();
    for token in normalized {
        match token.tok {
            Tok::Ident(name) => header.push(name),
            Tok::Dot => header.push(".".into()),
            Tok::Semi => break,
            _ => panic!("unexpected header token"),
        }
    }
    header.join(" ").replace(" . ", ".")
}

fn reject(source: &str, path: Option<&str>, code: &str) {
    let tokens = lex(source).unwrap();
    let error = namespace::normalize(&tokens, path.map(Path::new)).unwrap_err();
    assert_eq!(error.code, code, "{error}");
}

#[test]
fn derives_folder_and_parent_dot_folder_without_host_paths() {
    let path = "src/commerce/storefront/main.atto";
    assert_eq!(resolve("use default namespace;", path), "use namespace storefront");
    assert_eq!(resolve("use default namespace(1);", path), "use namespace storefront");
    assert_eq!(resolve("use default namespace(2);", path), "use namespace commerce.storefront");
    assert_eq!(resolve("use default namespace(3);", path), "use namespace src.commerce.storefront");
    assert_eq!(resolve("use namespace custom.app;", path), "use namespace custom.app");
}

#[test]
fn parsing_preserves_declarations_and_namespace_identity() {
    let tokens = lex("use default namespace(2);\nimport module math from \"./math.atto\";\ndeclare actor A;").unwrap();
    let h = target_syntax::parse_target_declarations_with_path(
        &tokens, Some(Path::new("src/commerce/storefront/main.atto"))).unwrap();
    assert_eq!(h.namespace.as_deref(), Some("commerce.storefront"));
    assert_eq!(h.imports, vec!["math"]);
    assert_eq!(h.actors, vec!["A"]);
}

#[test]
fn requires_path_and_rejects_invalid_depth_and_folder_names() {
    reject("use default namespace;", None, "E_NAMESPACE_PATH");
    reject("use default namespace(2);", Some("src/main.atto"), "E_NAMESPACE_DEPTH");
    reject("use default namespace(0);", Some("src/main.atto"), "E_NAMESPACE_DEPTH");
    reject("use default namespace(33);", Some("src/main.atto"), "E_NAMESPACE_DEPTH");
    reject("use default namespace(-1);", Some("src/main.atto"), "E_NAMESPACE_DEPTH");
    reject("use default namespace();", Some("src/main.atto"), "E_NAMESPACE_DEPTH");
    reject("use default namespace(1.5);", Some("src/main.atto"), "E_NAMESPACE_DEPTH");
    reject("use default namespace(2);", Some("src/invalid-name/main.atto"), "E_NAMESPACE_PATH");
    reject("use default namespace;", Some("../storefront/main.atto"), "E_NAMESPACE_PATH");
    reject("use default namespace;", Some("/tmp/storefront/main.atto"), "E_NAMESPACE_PATH");
    reject("use default namespace;", Some("storefront/main.txt"), "E_NAMESPACE_PATH");
    reject("use default namespace;", Some("main.atto"), "E_NAMESPACE_DEPTH");
}

#[test]
fn namespace_is_unique_first_and_terminated() {
    let path = Some("src/commerce/main.atto");
    reject("import module m from \"./m.atto\"; use default namespace;", path, "E_SYNTAX");
    reject("fnc main(): int {return 0;} use default namespace;", path, "E_SYNTAX");
    reject("use default namespace; use namespace override;", path, "E_DUPLICATE");
    reject("use namespace override; use default namespace;", path, "E_DUPLICATE");
    reject("use namespace x; use namespace y;", path, "E_DUPLICATE");
    reject("use default namespace(1)", path, "E_SYNTAX");
    reject("use namespace x.;", path, "E_SYNTAX");
}

#[test]
fn does_not_recognize_namespace_text_in_strings_or_comments() {
    let code = "pub fnc main(): int { stdio.println(\"use default namespace;\"); return 0; }";
    assert!(namespace::normalize(&lex(code).unwrap(), None).is_ok());
    let code = "// use default namespace;\npub fnc main(): int {return 0;}";
    assert!(namespace::normalize(&lex(code).unwrap(), None).is_ok());
}

#[test]
fn compiler_without_path_rejects_derived_header_before_emission() {
    let source = "use default namespace;\npub fnc main(): int { return 42; }";
    assert_eq!(atto_compiler::compile(source).unwrap_err().code, "E_NAMESPACE_PATH");
    // Path-aware target syntax is recognized, but external namespace linking
    // is still not implemented, so emitting a fake namespace-free guest is forbidden.
    assert_eq!(atto_compiler::compile_with_path(source,
        Path::new("src/storefront/main.atto")).unwrap_err().code, "E_UNSUPPORTED");
}
