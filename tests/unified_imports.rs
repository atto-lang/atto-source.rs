//! Contract tests for comma-separated static imports; no linker is implied.
use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;

fn parse(src: &str) -> atto_compiler::target_syntax::TargetHeaders {
    parse_target_declarations(&lex(src).expect("lex")).expect("parse")
}
fn reject(src: &str, code: &str) {
    let err = parse_target_declarations(&lex(src).expect("lex")).expect_err("must reject");
    assert_eq!(err.code, code, "{err}");
}

#[test]
fn static_namespace_imports_accept_files_and_packages() {
    let headers = parse(r#"
        use namespace storefront;
        import * as telemetry from pkg "core.telemetry";
        import * as local from "./lib.atto";
        import * as root from pkg "+/pkg";
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.namespace.as_deref(), Some("storefront"));
    assert_eq!(headers.imports, ["telemetry", "local", "root"]);
}

#[test]
fn namespace_is_optional_unique_and_before_every_import() {
    let with_namespace = parse(r#"
        // License header and comments may precede the namespace.
        use namespace storefront;
        import module math from "./math.atto";
        import span, foo as bar from pkg "core.telemetry";
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(with_namespace.namespace.as_deref(), Some("storefront"));
    assert_eq!(with_namespace.imports, ["math", "span", "bar"]);

    let without_namespace = parse("import sleep from pkg \"std.time\";");
    assert_eq!(without_namespace.namespace, None);
    assert_eq!(without_namespace.imports, ["sleep"]);

    for (src, code) in [
        ("import module math from \"./math.atto\"; use namespace storefront;", "E_SYNTAX"),
        ("use namespace storefront; import span from pkg \"core.telemetry\"; use namespace second;", "E_DUPLICATE"),
        ("use namespace storefront; use namespace storefront;", "E_DUPLICATE"),
        ("pub fnc main(): int { return 0; } use namespace storefront;", "E_SYNTAX"),
    ] {
        reject(src, code);
    }
}

#[test]
fn comma_separated_imports_support_names_and_aliases() {
    let headers = parse(r#"
        import span, foo from pkg "core.telemetry";
        import read as read_file, write as write_file from "./fs.atto";
        import sleep from pkg "std.time";
        import alpha as a, beta as b from pkg "+/my_pkg";
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.imports, [
        "span", "foo", "read_file", "write_file", "sleep", "a", "b"
    ]);
}

#[test]
fn dotted_static_export_paths_resolve_to_leaf_bindings_or_explicit_aliases() {
    let headers = parse(r#"
        import telemetry.span as span, telemetry.tracer as make_tracer, time.sleep from pkg "core";
        import foo.bar.baz as result from "./module.atto";
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.imports, ["span", "make_tracer", "sleep", "result"]);
}

#[test]
fn old_import_module_and_pkg_spelling_remain_parse_only_compatibility() {
    assert_eq!(parse("import module math from \"./math.atto\";").imports, ["math"]);
    assert_eq!(parse("import pkg p from \"+/pkg1\";").imports, ["p"]);
    // No guest module/package loader exists in this target-header path.
    assert!(atto_compiler::compile(
        "import foo from pkg \"core.telemetry\"; pub fnc main(): int { return 0; }"
    ).is_err());
}

#[test]
fn malformed_and_shadowing_imports_fail_closed() {
    for (src, code) in [
        ("import from pkg \"core\";", "E_SYNTAX"),
        ("import { a } from pkg \"core\";", "E_SYNTAX"),
        ("import { a, b } from pkg \"core\";", "E_SYNTAX"),
        ("import * from pkg \"core\";", "E_SYNTAX"),
        ("import a, from pkg \"core\";", "E_SYNTAX"),
        ("import , a from pkg \"core\";", "E_SYNTAX"),
        ("import a from pkg core;", "E_SYNTAX"),
        ("import a from pkg \"\";", "E_SYNTAX"),
        ("import a from pkg \"./local.atto\";", "E_SYNTAX"),
        ("import a from \"local.atto\";", "E_SYNTAX"),
        ("import a, a from pkg \"core\";", "E_DUPLICATE"),
        ("import a, b as a from pkg \"core\";", "E_DUPLICATE"),
        ("import x.a, y.a from pkg \"core\";", "E_DUPLICATE"),
        ("import * as a from pkg \"core\"; import x as a from pkg \"other\";", "E_DUPLICATE"),
        ("import a as from pkg \"core\";", "E_SYNTAX"),
        ("import a. from pkg \"core\";", "E_SYNTAX"),
        ("import a from pkg \"core\"", "E_SYNTAX"),
        ("declare actor Worker; import * as core from pkg \"core\";", "E_SYNTAX"),
        ("use core.telemetry;", "E_SYNTAX"),
    ] {
        reject(src, code);
    }
}

#[test]
fn dotted_paths_have_bounded_nesting() {
    let path = std::iter::repeat("a").take(9).collect::<Vec<_>>().join(".");
    reject(&format!("import {path} from pkg \"core\";"), "E_SOURCE_LIMIT");
}

#[test]
fn import_lists_obey_source_bounds() {
    let names = (0..257).map(|n| format!("n{n}")).collect::<Vec<_>>().join(", ");
    reject(&format!("import {names} from pkg \"core\";"), "E_SOURCE_LIMIT");
}
