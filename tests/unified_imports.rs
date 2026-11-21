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
        ("fnc init(): void {} import module math from \"./math.atto\";", "E_SYNTAX"),
        ("pub fnc main(): int { return 0; } import module math from \"./math.atto\";", "E_SYNTAX"),
        ("use namespace storefront; pub fnc main(): int { return 0; } import x from pkg \"core\";", "E_SYNTAX"),
        ("routine background(): void {} import x from pkg \"core\";", "E_SYNTAX"),
        ("define actor Worker as end import x from pkg \"core\";", "E_SYNTAX"),
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
fn import_bindings_never_shadow_file_scope_declarations() {
    for src in [
        "import Worker from pkg \"core\"; declare actor Worker;",
        "import Widget from pkg \"core\"; declare class Widget;",
        "import Item from pkg \"core\"; declare struct Item;",
        "import make as run from pkg \"core\"; pub fnc run(): int { return 0; }",
        "import calculate from pkg \"core\"; int <- calculate() { return 0; }",
        "import Worker from pkg \"core\"; define actor Worker as end",
    ] {
        reject(src, "E_DUPLICATE");
    }
}

#[test]
fn static_imports_must_precede_file_scope_definitions() {
    reject("pub fnc main(): int { return 0; } import foo from pkg \"core\";", "E_SYNTAX");
    reject("declare actor Worker; import foo from pkg \"core\";", "E_SYNTAX");
    reject("define class Foo as end import foo from pkg \"core\";", "E_SYNTAX");
}

#[test]
fn import_names_may_coexist_with_unrelated_methods_and_local_names() {
    let headers = parse(r#"
        import foo from pkg "core";
        declare actor Worker;
        mtd (Worker self) foo(): void {}
        pub fnc main(): int { val foo = 42; return foo; }
    "#);
    assert_eq!(headers.imports, ["foo"]);
    assert_eq!(headers.receivers.len(), 1);
}


#[test]
fn canonical_package_shorthand_records_static_local_bindings_only() {
    let headers = parse(r#"
        use namespace storefront;
        import pkg 'z';
        import pkg 'core.telemetry' as tracer;
        import pkg '@scope/math' as math;
        import pkg '+/utils' as utils;
        import * as local from './helpers.atto';
        pub fnc main(): int { return 0; }
    "#);
    assert_eq!(headers.namespace.as_deref(), Some("storefront"));
    assert_eq!(headers.imports, ["z", "tracer", "math", "utils", "local"]);
}

#[test]
fn canonical_package_imports_reject_implicit_bad_names_and_collisions() {
    for src in [
        "import pkg 'my-lib';",
        "import pkg '@scope/math';",
        "import pkg '+/utils';",
        "import pkg 'core.class';",
        "import pkg '';",
        "import pkg 'core..math' as math;",
        "import pkg 'z' as class;",
        "import pkg z;",
        "import pkg 'z' as;",
        "import pkg 'z'",
        "import pkg './local' as local;",
    ] {
        reject(src, "E_SYNTAX");
    }
    for src in [
        "import pkg 'z'; import pkg 'other' as z;",
        "import pkg 'z'; import * as z from './local.atto';",
    ] {
        reject(src, "E_DUPLICATE");
    }
}

#[test]
fn canonical_pkg_imports_remain_unlinked_in_executable_relative_file_compiler() {
    use atto_compiler::compile_with_imports;
    use std::{io, path::Path};
    for src in [
        "import pkg 'z'; pub fnc main():int{return 0;}",
        "import pkg 'z' as zed; pub fnc main():int{return 0;}",
    ] {
        let failure = compile_with_imports(src, Path::new("project/main.atto"), |_path| {
            Err(io::Error::new(io::ErrorKind::NotFound, "no package imports"))
        }).expect_err("unverified package source must not produce Wasm");
        assert_eq!(failure.code, "E_UNSUPPORTED", "{failure}");
    }
}


#[test]
fn canonical_atto_libraries_must_not_load_from_guest_filesystem() {
    use atto_compiler::compile_with_imports;
    use std::path::Path;

    for spec in [
        "atto.core/numeric",
        "atto.core/numeric.atto",
        "atto.alloc/byte_region",
        "atto.std/fs",
        "atto.runtime.actor",
        "./atto.core/numeric.atto",
        "../atto.std/fs.atto",
        "./vendor/atto.alloc/byte_region.atto",
    ] {
        let src = format!(
            "import * as library from \"{spec}\"; pub fnc main(): int {{ return 0; }}"
        );
        let mut loaded = false;
        let err = compile_with_imports(&src, Path::new("project/main.atto"), |_| {
            loaded = true;
            panic!("reserved library names must be denied before host file I/O")
        })
        .expect_err("unsupported library/profile must not emit Wasm");
        assert_eq!(err.code, "E_STDLIB_UNLINKED", "{spec}: {err}");
        assert!(!loaded, "{spec} touched the external source loader");
    }

    // The normal one-hop local source import remains a separate capability,
    // not a blanket ban on every relative file import.
    let regular = r#"import * as helpers from "./helpers.atto"; pub fnc main(): int { return helpers.answer(); }"#;
    let mut called = false;
    let _ = compile_with_imports(regular, Path::new("project/main.atto"), |_| {
        called = true;
        Ok("pub fnc answer(): int { return 42; }".to_string())
    });
    assert!(called, "ordinary relative source import should still reach the loader");
}
