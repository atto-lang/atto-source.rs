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
    // With a logical path the derived namespace is plain file identity: a
    // standalone file compiles to a real guest (no import authority granted).
    let wasm = atto_compiler::compile_with_path(source, Path::new("src/storefront/main.atto"))
        .expect("derived namespace is file identity only");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn v2_file_named_and_folder_modes_are_distinct() {
    use namespace::{resolve_mode_v2, NamespaceModeV2::*};
    let path = Some(Path::new("src/network/http/client.atto"));
    let mode = |src: &str, path| resolve_mode_v2(&lex(src).unwrap(), path).unwrap();
    for src in ["", "use default namespace;", "use file namespace;"] {
        assert_eq!(mode(src, path), File);
        assert_eq!(mode(src, None), File);
    }
    assert_eq!(mode("use folder namespace;", path),
        Folder(vec!["src".into(), "network".into(), "http".into()]));
    assert_eq!(mode("use folder namespace;", Some(Path::new("main.atto"))), Folder(vec![]));
    assert_eq!(mode("use default namespace(1);", path), mode("use namespace http;", path));
    assert_eq!(mode("use default namespace(2);", path), mode("use namespace network.http;", path));
    assert_eq!(mode("use default namespace(3);", path), mode("use namespace src.network.http;", path));
    assert_ne!(mode("use folder namespace;", path), mode("use default namespace(3);", path));
    assert_ne!(mode("use folder namespace;", path), mode("use folder namespace;", Some(Path::new("src/db/http/client.atto"))));
}

#[test]
fn v2_rejects_ambiguous_paths_mixed_headers_and_depth_errors() {
    use namespace::resolve_mode_v2;
    for path in ["../http/main.atto", "/src/http/main.atto", "src/./http/main.atto",
                 "src//http/main.atto", "src/http/main.txt", "src/http/.atto",
                 "src\\http\\main.atto"] {
        assert_eq!(resolve_mode_v2(&lex("use folder namespace;").unwrap(), Some(Path::new(path)))
            .unwrap_err().code, "E_NAMESPACE_PATH", "{path}");
    }
    assert_eq!(resolve_mode_v2(&lex("use folder namespace;").unwrap(), None)
        .unwrap_err().code, "E_NAMESPACE_PATH");
    let path = Some(Path::new("src/http/main.atto"));
    for src in ["use default namespace(0);", "use default namespace(33);",
                "use default namespace(3);", "use default namespace(-1);"] {
        assert_eq!(resolve_mode_v2(&lex(src).unwrap(), path).unwrap_err().code, "E_NAMESPACE_DEPTH");
    }
    for src in ["use file namespace; use folder namespace;",
                "use namespace x; use file namespace;", "use default namespace; use default namespace(1);"] {
        assert_eq!(resolve_mode_v2(&lex(src).unwrap(), path).unwrap_err().code, "E_DUPLICATE");
    }
    for src in ["use file namespace", "use folder namespace(1);", "import module m from \"./m.atto\"; use file namespace;"] {
        assert_eq!(resolve_mode_v2(&lex(src).unwrap(), path).unwrap_err().code, "E_SYNTAX");
    }
}

#[test]
fn opt_in_file_v2_compiles_all_three_spellings_through_real_emitters() {
    use atto_compiler::{compile_namespace_file_v2, compile_js_namespace_file_v2};
    let body = "pub fnc main(): int { return 42; }";
    let baseline = format!("\n\n{body}");
    let wasm = compile_namespace_file_v2(&baseline).unwrap();
    let js = compile_js_namespace_file_v2(&baseline).unwrap();
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    for header in ["use default namespace;", "use file namespace;", "use /* é */\n file namespace;"] {
        let source = if header.contains('\n') { format!("{header}\n{body}") }
            else { format!("{header}\n\n{body}") };
        assert_eq!(compile_namespace_file_v2(&source).unwrap(), wasm);
        assert_eq!(compile_js_namespace_file_v2(&source).unwrap(), js);
    }
}

#[test]
fn opt_in_file_v2_never_falls_back_to_a_shared_namespace_or_erases_errors() {
    use atto_compiler::compile_namespace_file_v2;
    for (source, code) in [
        ("use namespace x; pub fnc main(): int { return 42; }", "E_UNSUPPORTED"),
        ("use folder namespace; pub fnc main(): int { return 42; }", "E_NAMESPACE_PATH"),
        ("use default namespace(1); pub fnc main(): int { return 42; }", "E_NAMESPACE_PATH"),
        ("use file namespace; use default namespace; pub fnc main(): int { return 42; }", "E_DUPLICATE"),
        ("use file namespace pub fnc main(): int { return 42; }", "E_SYNTAX"),
    ] { assert_eq!(compile_namespace_file_v2(source).unwrap_err().code, code); }
    assert!(compile_namespace_file_v2("use file namespace; pub fnc main(): int { return true; }").is_err());
    assert!(compile_namespace_file_v2("use file namespace; declare class Foo; pub fnc main(): int { return 42; }").is_err());
}
