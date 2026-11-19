//! Contract tests for the static import grammar. No import linker is implied.
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
fn static_named_nested_and_renamed_exports_are_parsed_as_bindings() {
    let headers = parse(r#"
        import { span, tracer as create_tracer } from pkg "core.telemetry";
        import { calculate, Config } from "./lib.atto";
        import { telemetry: { writer, trace: { open as begin_trace } }, time: { sleep } } from pkg "core";
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.imports, [
        "span", "create_tracer", "calculate", "Config", "writer", "begin_trace", "sleep"
    ]);
}

#[test]
fn old_import_module_and_pkg_spelling_remain_parse_only_compatibility() {
    assert_eq!(parse("import module math from \"./math.atto\";").imports, ["math"]);
    assert_eq!(parse("import pkg p from \"+/pkg1\";").imports, ["p"]);
    // No guest module/package loader exists in this target-header path.
    assert!(atto_compiler::compile(
        "import { foo } from pkg \"core.telemetry\"; pub fnc main(): int { return 0; }"
    ).is_err());
}

#[test]
fn malformed_and_shadowing_imports_fail_closed() {
    for (src, code) in [
        ("import {} from pkg \"core\";", "E_SYNTAX"),
        ("import { a: {} } from pkg \"core\";", "E_SYNTAX"),
        ("import * from pkg \"core\";", "E_SYNTAX"),
        ("import { a } pkg \"core\";", "E_SYNTAX"),
        ("import { a } from pkg core;", "E_SYNTAX"),
        ("import { a } from pkg \"\";", "E_SYNTAX"),
        ("import { a } from pkg \"./local.atto\";", "E_SYNTAX"),
        ("import { a, a } from pkg \"core\";", "E_DUPLICATE"),
        ("import { a, b as a } from pkg \"core\";", "E_DUPLICATE"),
        ("import { x: { a }, y: { a } } from pkg \"core\";", "E_DUPLICATE"),
        ("import * as a from pkg \"core\"; import { x as a } from pkg \"other\";", "E_DUPLICATE"),
        ("import { a as } from pkg \"core\";", "E_SYNTAX"),
        ("import { a } from pkg \"core\"", "E_SYNTAX"),
        ("declare actor Worker; import * as core from pkg \"core\";", "E_SYNTAX"),
        ("use core.telemetry;", "E_SYNTAX"),
    ] {
        reject(src, code);
    }
}

#[test]
fn selectors_have_bounded_nesting() {
    let mut source = String::from("import { ");
    for _ in 0..9 { source.push_str("a: { "); }
    source.push_str("x");
    for _ in 0..9 { source.push_str(" }"); }
    source.push_str(" } from pkg \"core\";");
    reject(&source, "E_SOURCE_LIMIT");
}
