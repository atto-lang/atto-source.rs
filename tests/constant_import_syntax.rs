//! Parse-only coverage for typed constant imports. This test does not link packages.
use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;

fn parse(src: &str) -> atto_compiler::target_syntax::TargetHeaders {
    parse_target_declarations(&lex(src).expect("lex")).expect("target declaration parse")
}
fn reject(src: &str, expected: &str) {
    let err = parse_target_declarations(&lex(src).expect("lex")).expect_err("must reject");
    assert_eq!(err.code, expected, "{src}\n{err}");
}

#[test]
fn named_const_imports_support_dotted_packages_pkg_and_relative_files() {
    let headers = parse(r#"
        use namespace app;
        import const status_valid, route_matches as route_match from "std.http";
        import const status_not_found as missing from pkg "std.http";
        import const threshold from "./config.atto";
        pub fnc main(): int { return status_valid; }
    "#);
    assert_eq!(headers.namespace.as_deref(), Some("app"));
    assert_eq!(
        headers.imports,
        ["status_valid", "route_match", "missing", "threshold"]
    );
    assert_eq!(headers.const_imports, headers.imports);
}

#[test]
fn file_constants_are_static_headers_with_private_default_and_pub_variant() {
    let headers = parse(r#"
        use namespace std.http;
        define const int internal_code = 123;
        define pub const int status_valid = 200;
        define pub const int status_not_found as 404;
        define pub const string status_from_env = std.env.read_env("status_valid");
        define pub const string host = os.read_env("HOST");
        pub fnc main(): int { return status_valid; }
    "#);
    assert_eq!(
        headers.constants,
        ["internal_code", "status_valid", "status_not_found", "status_from_env", "host"]
    );
    assert!(headers.imports.is_empty());
    assert!(headers.const_imports.is_empty());
    assert_eq!(headers.env_const_reads, ["status_from_env", "host"]);
}

#[test]
fn canonical_qualified_std_env_read_in_namespace_is_a_static_const_shape() {
    let headers = parse(r#"
        use namespace std.http;
        define pub const string status_valid = std.env.read_env("status_valid");
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.namespace.as_deref(), Some("std.http"));
    assert_eq!(headers.constants, ["status_valid"]);
    assert_eq!(headers.env_const_reads, ["status_valid"]);

    // A key-name identifier is only syntactically admitted; the typed
    // constexpr checker must separately establish its constant provenance.
    let id_key = parse(r#"
        define const string key = "ATTO_KEY";
        define pub const string value = std.env.read_env(key);
    "#);
    assert_eq!(id_key.env_const_reads, ["value"]);
}

#[test]
fn malformed_direct_static_environment_reads_fail_early() {
    for source in [
        r#"define pub const string x = std.env.read_env();"#,
        r#"define pub const string x = std.env.read_env("A", "B");"#,
        r#"define pub const string x = std.env.read_env(123);"#,
        r#"define pub const string x = std.env.read_env("A").field;"#,
        r#"define pub const string x = os.read_env("A", "B");"#,
    ] {
        reject(source, "E_CONST");
    }
}

#[test]
fn typed_and_untyped_imports_share_the_same_collision_domain() {
    reject(
        "import const ok from \"std.http\"; import ok from pkg \"other.pkg\";",
        "E_DUPLICATE"
    );
    reject(
        "import const ok as local from \"std.http\"; import const fail as local from \"other.pkg\";",
        "E_DUPLICATE"
    );
    reject(
        "import const status_valid from \"std.http\"; define const int status_valid = 200;",
        "E_DUPLICATE"
    );
    reject(
        "import const run from \"std.http\"; pub fnc run(): int { return 0; }",
        "E_DUPLICATE"
    );
    reject(
        "define const int duplicate = 1; define const int duplicate = 2;",
        "E_DUPLICATE"
    );
}

#[test]
fn invalid_const_import_forms_fail_closed() {
    for source in [
        "import const route_matches as matches from \"std.http\";",
        "import const * as http from \"std.http\";",
        "import const from \"std.http\";",
        "import const a, from \"std.http\";",
        "import const a from \"std..http\";",
        "import const a from \"missing.atto\";",
        "import const a from \"std/http\";",
        "import const a from pkg \"./relative.atto\";",
        "import const { a } from \"std.http\";",
    ] {
        reject(source, "E_SYNTAX");
    }
}

#[test]
fn immutable_top_level_const_is_distinct_from_forbidden_global_locals() {
    for source in [
        "let code: int = 200;",
        "val code: int = 200;",
        "var code: int = 200;",
        "define let int code = 200;",
        "define const mut int code = 200;",
        "define const int code = ;",
        "define const int code;",
        "define const int code = (1 + 2;",
    ] {
        let err = parse_target_declarations(&lex(source).expect("lex"))
            .expect_err("top-level mutable or malformed declaration must fail");
        assert!(
            matches!(err.code, "E_SYNTAX" | "E_EOF"),
            "{source}: {err}"
        );
    }
}

#[test]
fn parser_only_does_not_claim_executable_package_linker() {
    // Executable package imports require a separate typed symbol resolver.
    let source = "import const status_valid from \"std.http\"; pub fnc main(): int { return status_valid; }";
    assert!(atto_compiler::compile(source).is_err());
}
