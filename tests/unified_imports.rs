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

#[test]
fn scoped_using_timeout_await_demo_has_statically_parsed_named_imports() {
    // The target-header parser checks only the imports. It must not claim
    // that the nested scope body is executable or validated by this stage.
    let headers = parse(r#"
        import span from pkg "core.telemetry";
        import sleep from pkg "std.time";
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.imports, ["span", "sleep"]);
}

#[test]
fn scoped_using_timeout_await_demo_must_not_produce_guest_without_runtime_support() {
    // Exact /play surface form: this is an intentionally pending executable
    // gate, not a simulated success or a test of a source-string interpreter.
    let source = r#"
import span from pkg "core.telemetry";
import sleep from pkg "std.time";
pub suspends fnc main(): int {
  using span("playground.demo") as current_span {
    with timeout(2000) {
      current_span.set_attribute("feature", "import");
      await sleep(1);
      stdio.println("span retained over suspension");
    }
  }
  return 0;
}
"#;
    let rejected = atto_compiler::compile(source)
        .expect_err("must fail closed until imports, using, timeout and await actually lower");
    assert!(!rejected.code.is_empty(), "compiler must explain rejection");
}
