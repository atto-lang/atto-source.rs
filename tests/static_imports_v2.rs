//! v2 syntax and bounded real compiler lane. No package/URL guest support claimed.
use atto_compiler::{compile, compile_with_imports, import_v2::{Kind, Source}, lexer::lex,
    target_syntax::parse_target_declarations};
use std::path::Path;

#[test]
fn mixed_delimiters_and_explicit_groups_are_equivalent() {
    let a = parse_target_declarations(&lex("import (class C, fnc f, (module x, y, z)) from pkg '@foo/bar/baz';").unwrap()).unwrap();
    let b = parse_target_declarations(&lex("import [class C, fnc f, (module x, y, z)] from pkg '@foo/bar/baz';").unwrap()).unwrap();
    assert_eq!(a.mixed_imports, b.mixed_imports);
    assert_eq!(a.imports, ["C", "f", "x", "y", "z"]);
    let m = &a.mixed_imports[0];
    assert_eq!(m.source, Source::Package("@foo/bar/baz".into()));
    assert_eq!(m.selectors.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [Kind::Class, Kind::Fnc, Kind::Module, Kind::Module, Kind::Module]);
}

#[test]
fn aliases_and_source_domains_survive_parse() {
    let a = parse_target_declarations(&lex("import ((class C, D as Data), fnc f as fetch, const N, namespace http as vendor.http,) from std.num;").unwrap()).unwrap();
    assert_eq!(a.imports, ["C", "Data", "fetch", "N", "vendor.http"]);
    assert_eq!(a.const_imports, ["N"]);
    assert_eq!(a.mixed_imports[0].source, Source::Builtin("std.num".into()));
    let b = parse_target_declarations(&lex("import (fnc f) from './lib.atto';").unwrap()).unwrap();
    assert_eq!(b.mixed_imports[0].source, Source::Local("./lib.atto".into()));
    let c = parse_target_declarations(&lex("import (namespace remote) from pkg 'https://example.test/mod.atto';").unwrap()).unwrap();
    assert_eq!(c.mixed_imports[0].source, Source::Package("https://example.test/mod.atto".into()));
}

#[test]
fn malformed_lists_and_duplicate_bindings_fail() {
    for src in [
        "import (class C, fnc f, module x, y) from pkg '@foo/bar/baz';",
        "import [class C, fnc f, module x, y] from pkg '@foo/bar/baz';",
        "import (class C, (module x, fnc y)) from pkg '@foo/bar/baz';",
        "import ((module)) from pkg '@foo/bar/baz';",
        "import (class C, (module x, y,)) from pkg '@foo/bar/baz';",
        "import (class C] from pkg '@foo/bar/baz';",
        "import () from pkg '@foo/bar/baz';",
        "import (class C, fnc f as C) from pkg '@foo/bar/baz';",
        "import (class C) from pkg '@foo/bar/baz'; import (fnc C) from './lib.atto';",
        "import (fnc f) from pkg './lib.atto';",
        "import (fnc f) from foo.bar;",
    ] {
        assert!(parse_target_declarations(&lex(src).unwrap()).is_err(), "{src}");
    }
}

#[test]
fn unresolved_mixed_import_never_loads_a_dependency_or_emits_wasm() {
    let src = "import (class C, (fnc f, g)) from './lib.atto'; pub fnc main(): int { return 42; }";
    let mut calls = 0;
    let error = compile_with_imports(src, Path::new("main.atto"), |_| {
        calls += 1;
        Ok("pub fnc f(): int { return 1; }".into())
    }).unwrap_err();
    assert_eq!(error.code, "E_UNSUPPORTED");
    assert_eq!(calls, 0);
    assert_eq!(compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn natural_builtin_namespace_uses_the_real_existing_wasm_emitter() {
    let tail = "pub fnc main(): int { return num.clamp(42, 0, 100); }";
    let legacy = compile(&format!("import module num from 'core/num'; {tail}")).unwrap();
    assert_eq!(&legacy[..4], b"\0asm");
    for selector in ["core.num", "'core.num'", "\"core.num\""] {
        let current = compile(&format!("import namespace num from {selector}; {tail}")).unwrap();
        assert_eq!(legacy, current, "{selector}");
    }
}

#[test]
fn builtin_domain_never_delegates_to_file_loader() {
    for src in [
        "import namespace missing from core.missing;",
        "import namespace num from pkg 'core.num';",
        "import namespace num from pkg 'core/num';",
        "import namespace num from core.num; import namespace num from 'core.num';",
        "import namespace alias from core.num;",
    ] {
        let src = format!("{src} pub fnc main(): int {{ return 42; }}");
        let mut calls = 0;
        assert!(compile_with_imports(&src, Path::new("main.atto"), |_| {
            calls += 1;
            Ok(String::new())
        }).is_err());
        assert_eq!(calls, 0, "{src}");
    }
}

#[test]
fn kind_first_groups_and_singletons_bind_equivalently() {
    let forms = [
        "import class (C, D), fnc (f, g), module m, namespace (z) from pkg '@a/b/c';",
        "import (class (C, D), fnc (f, g), module m, namespace (z)) from pkg '@a/b/c';",
        "import [(class C, D), (fnc f, g), module m, (namespace z)] from pkg '@a/b/c';",
        "import (class C, D), (fnc f, g), module m, (namespace z) from pkg '@a/b/c';",
        "import class C, class D, fnc f, fnc g, module m, namespace z from pkg '@a/b/c';",
    ];
    let first = parse_target_declarations(&lex(forms[0]).unwrap()).unwrap();
    assert_eq!(first.imports, ["C", "D", "f", "g", "m", "z"]);
    for src in forms {
        let actual = parse_target_declarations(&lex(src).unwrap()).unwrap();
        assert_eq!(actual.mixed_imports, first.mixed_imports, "{src}");
        let mut calls = 0;
        assert_eq!(compile_with_imports(src, Path::new("main.atto"), |_| {
            calls += 1; Ok(String::new())
        }).unwrap_err().code, "E_UNSUPPORTED");
        assert_eq!(calls, 0);
    }
    for src in ["import namespace (z) from pkg '@a/b/c';",
                "import ((namespace z)) from pkg '@a/b/c';"] {
        let actual = parse_target_declarations(&lex(src).unwrap()).unwrap();
        assert_eq!(actual.imports, ["z"]);
        assert_eq!(actual.mixed_imports[0].selectors[0].kind, Kind::Namespace);
    }
    for src in [
        "import class (), fnc f from pkg '@a/b/c';",
        "import class (C, fnc f) from pkg '@a/b/c';",
        "import class (C, D), f from pkg '@a/b/c';",
        "import class C, fnc f, g from pkg '@a/b/c';",
        "import class (C, D), namespace (z), from pkg '@a/b/c';",
        "import namespace (z), fnc (f as z) from pkg '@a/b/c';",
        "import class (C, D,) from pkg '@a/b/c';",
        "import class ((C)) from pkg '@a/b/c';",
    ] {
        assert!(parse_target_declarations(&lex(src).unwrap()).is_err(), "{src}");
    }
}

#[test]
fn brackets_and_parentheses_are_interchangeable_at_each_group_position() {
    let forms = [
        "import class [C, D as Data], fnc [f as fetch, g], module (m, n), namespace (z) from pkg '@a/b/c';",
        "import class (C, D as Data), fnc (f as fetch, g), module [m, n], namespace [z] from pkg '@a/b/c';",
        "import (class [C, D as Data], fnc (f as fetch, g), [module m, n], namespace [z]) from pkg '@a/b/c';",
        "import [(class C, D as Data), fnc [f as fetch, g], module (m, n), [namespace z]] from pkg '@a/b/c';",
        "import [class C, D as Data], [fnc f as fetch, g], module [m, n], [namespace z] from pkg '@a/b/c';",
    ];
    let expected = parse_target_declarations(&lex(forms[0]).unwrap()).unwrap();
    assert_eq!(expected.imports, ["C", "Data", "fetch", "g", "m", "n", "z"]);
    for src in forms {
        let actual = parse_target_declarations(&lex(src).unwrap()).unwrap();
        assert_eq!(actual.mixed_imports, expected.mixed_imports, "{src}");
        let mut loads = 0;
        assert_eq!(compile_with_imports(src, Path::new("main.atto"), |_| {
            loads += 1; Ok(String::new())
        }).unwrap_err().code, "E_UNSUPPORTED");
        assert_eq!(loads, 0, "{src}");
    }
    for src in ["import namespace [z] from pkg '@a/b/c';",
                "import [namespace z] from pkg '@a/b/c';",
                "import [[namespace z]] from pkg '@a/b/c';"] {
        assert_eq!(parse_target_declarations(&lex(src).unwrap()).unwrap().imports, ["z"]);
    }
    for src in [
        "import class [C, D) from pkg '@a/b/c';",
        "import class (C, D] from pkg '@a/b/c';",
        "import class [] from pkg '@a/b/c';",
        "import class [C, fnc f] from pkg '@a/b/c';",
        "import class [C, D,] from pkg '@a/b/c';",
        "import class [[C]] from pkg '@a/b/c';",
        "import class [C], fnc [f as C] from pkg '@a/b/c';",
        "import class [C], g from pkg '@a/b/c';",
    ] {
        assert!(parse_target_declarations(&lex(src).unwrap()).is_err(), "{src}");
    }
}

fn parse_v2(src: &str) -> atto_compiler::target_syntax::TargetHeaders {
    parse_target_declarations(&lex(src).unwrap()).unwrap()
}
fn reject_v2(src: &str, expected: &str) {
    let d = parse_target_declarations(&lex(src).unwrap()).unwrap_err();
    assert_eq!(d.code, expected, "{src}: {d}");
}

#[test]
fn fallback_keeps_precise_duplicate_and_limit_diagnostics() {
    for src in [
        "import (class C, D as C) from pkg '@a/b/c';",
        "import [class C, D as C] from pkg '@a/b/c';",
        "import class [C, D as C] from pkg '@a/b/c';",
        "import (class C, fnc f as C) from pkg '@a/b/c';",
    ] {
        reject_v2(src, "E_DUPLICATE");
    }
    for path in ["a.b.c.d.e.f.g.h.i", "a.b.c.d.e.f.g.h.i as z"] {
        reject_v2(&format!("import [class {path}] from pkg '@a/b/c';"), "E_SOURCE_LIMIT");
    }
}

#[test]
fn explicit_non_namespace_prefixes_conflict_in_both_orders() {
    for src in [
        "import fnc [f], namespace [x as f.child] from pkg '@a/b/c';",
        "import namespace [x as f.child], fnc [f] from pkg '@a/b/c';",
        "import fnc [f] from pkg '@a/b/c'; import namespace [x as f.child] from pkg '@a/b/d';",
        "import namespace [x as f.child] from pkg '@a/b/d'; import fnc [f] from pkg '@a/b/c';",
        "import namespace [x as f.child] from pkg '@a/b/c'; pub fnc f(): int { return 1; }",
        "import namespace [x as f.child] from pkg '@a/b/c'; define const int f = 1;",
        "import namespace [x as C.child] from pkg '@a/b/c'; declare class C;",
    ] {
        reject_v2(src, "E_DUPLICATE");
    }
    let headers = parse_v2("import namespace [x as org.a, y as org.b], fnc [orphan] from pkg '@a/b/c';");
    assert_eq!(headers.imports, ["org.a", "org.b", "orphan"]);
    let headers = parse_v2("import namespace [x as foobar.child], fnc [foo] from pkg '@a/b/c';");
    assert_eq!(headers.imports, ["foobar.child", "foo"]);
}

#[test]
fn per_file_binding_budget_cannot_be_evaded_by_splitting_lists() {
    let group = |start: usize, end: usize| {
        let names = (start..end).map(|i| format!("C{i}")).collect::<Vec<_>>().join(", ");
        format!("import class [{names}] from pkg '@a/b/c';")
    };
    let exact = format!("{} {}", group(0, 128), group(128, 256));
    assert_eq!(parse_v2(&exact).imports.len(), 256);
    reject_v2(&format!("{exact} {}", group(256, 257)), "E_SOURCE_LIMIT");
    reject_v2(&group(0, 257), "E_SOURCE_LIMIT");
    reject_v2(&format!("{exact} import z from './lib.atto';"), "E_SOURCE_LIMIT");
}

#[test]
fn decoded_source_controls_and_package_path_spoofs_fail_before_loading() {
    for src in [
        r"import fnc [f] from './lib\u{0}.atto';",
        r"import fnc [f] from pkg '@a/b/c\n';",
        r"import fnc [f] from pkg '@a/b/c\t';",
        r"import fnc [f] from pkg '@a/b/c\\d';",
    ] {
        reject_v2(src, "E_IMPORT_PATH");
        let mut loads = 0;
        assert_eq!(compile_with_imports(src, Path::new("main.atto"), |_| {
            loads += 1; Ok(String::new())
        }).unwrap_err().code, "E_IMPORT_PATH");
        assert_eq!(loads, 0);
    }
    reject_v2("import fnc [f] from pkg '/tmp/lib.atto';", "E_SYNTAX");
    for selector in ["core.num", "'core.num'", "\"core.num\"", "'core/num'", "'atto.core/num.atto'"] {
        let headers = parse_v2(&format!("import fnc [clamp] from {selector};"));
        assert_eq!(headers.mixed_imports[0].source, Source::Builtin("core.num".into()));
    }
    let external = parse_v2("import fnc [clamp] from pkg 'core.num';");
    assert_eq!(external.mixed_imports[0].source, Source::Package("core.num".into()));
}

#[test]
fn all_delimiter_combinations_produce_same_typed_ir() {
    let expected = parse_v2("import class [C, D as Data], fnc [f, g], namespace [z] from pkg '@a/b/c';");
    let delimiters = [("(", ")"), ("[", "]")];
    for (outer_l, outer_r) in [("", ""), ("(", ")"), ("[", "]")] {
        for (cl, cr) in delimiters {
            for (fl, fr) in delimiters {
                for (nl, nr) in delimiters {
                    for kind_inside in [false, true] {
                        let group = |kind: &str, names: &str, left: &str, right: &str| {
                            if kind_inside { format!("{left}{kind} {names}{right}") }
                            else { format!("{kind} {left}{names}{right}") }
                        };
                        let src = format!("import {outer_l}{}, {}, {}{outer_r} from pkg '@a/b/c';",
                            group("class", "C, D as Data", cl, cr),
                            group("fnc", "f, g", fl, fr),
                            group("namespace", "z", nl, nr));
                        let actual = parse_v2(&src);
                        assert_eq!(actual.mixed_imports, expected.mixed_imports, "{src}");
                    }
                }
            }
        }
    }
}

#[test]
fn malformed_bare_builtins_never_fall_back_to_local_loader() {
    for selector in ["core", "core.num.extra", "core.num.", "core.num.atto", "core..num", "core/num", "'core.num.extra'"] {
        let src = format!("import namespace num from {selector}; pub fnc main(): int {{ return 0; }}");
        let mut loads = 0;
        assert!(compile_with_imports(&src, Path::new("main.atto"), |_| {
            loads += 1; Ok(String::new())
        }).is_err(), "{src}");
        assert_eq!(loads, 0, "{src}");
    }
}

#[test]
fn malformed_later_clause_prevents_partial_import_loading() {
    for second in [
        "import class [C, D) from pkg '@a/b/c';",
        "import (fnc f, class C as f) from pkg '@a/b/c';",
        "import namespace [z] from pkg '@a/b/c';",
    ] {
        let src = format!("import f from './lib.atto'; {second} pub fnc main(): int {{ return 42; }}");
        let mut loads = 0;
        assert!(compile_with_imports(&src, Path::new("main.atto"), |_| {
            loads += 1; Ok("pub fnc f(): int { return 0; }".into())
        }).is_err());
        assert_eq!(loads, 0, "{src}");
    }
}

#[test]
fn six_builtin_namespace_spellings_compile_and_bind_only_admitted_names() {
    let cases: [(&str, &[&str], &[&str]); 6] = [
        ("import namespace num from core.num;", &["num", "core.num"], &["other"]),
        ("import num from core.num;", &["num", "core.num"], &["other"]),
        ("import core.num from core.num;", &["core.num"], &["num"]),
        ("import core.num as num;", &["num"], &["core.num"]),
        ("import core.num as core.num;", &["core.num"], &["num"]),
        ("import core.num;", &["num", "core.num"], &["other"]),
    ];
    let baseline = compile("import module num from 'core/num'; pub fnc main(): int { return num.clamp(42, 0, 100); }").unwrap();
    for (import, allowed, denied) in cases {
        for binding in allowed {
            let source = format!("{import} pub fnc main(): int {{ return {binding}.clamp(42, 0, 100); }}");
            assert_eq!(compile(&source).unwrap(), baseline, "{source}");
        }
        for binding in denied {
            let source = format!("{import} pub fnc main(): int {{ return {binding}.clamp(42, 0, 100); }}");
            assert!(compile(&source).is_err(), "{source}");
        }
    }
}

#[test]
fn builtin_aliases_are_static_and_cannot_capture_runtime_bindings() {
    for source in [
        "import core.num as arithmetic; pub fnc main(): int { return arithmetic.clamp(42, 0, 100); }",
        "import core.num as custom.arithmetic; pub fnc main(): int { return custom.arithmetic.clamp(42, 0, 100); }",
        "import core.num; pub fnc main(): int { return core /* comment */ . num.clamp(42, 0, 100); }",
        "import core.num; pub fnc main(): int { return core.num.clamp(42, 0, 100); }",
    ] {
        assert!(compile(source).is_ok(), "{source}");
    }
    for source in [
        "import core.num as math; pub fnc main(): int { return num.clamp(42, 0, 100); }",
        "import core.num as math; pub fnc main(): int { let int math = 2; return math.clamp(42, 0, 100); }",
        "import core.num; pub fnc main(): int { let int core = 2; return core.num.clamp(42, 0, 100); }",
        "import core.num; pub fnc main(): int { return core.num; }",
        "import core.num; pub fnc main(): int { return object.core.num.clamp(42, 0, 100); }",
        "import core.num; import core.num as num; pub fnc main(): int { return 42; }",
        "import core.num as root; import core.cmp as root.cmp; pub fnc main(): int { return 42; }",
        "import core.num; import pkg '@acme/math/num'; pub fnc main(): int { return 42; }",
        "import core.num as __atto_internal_module_num; pub fnc main(): int { return 42; }",
        "import core.missing; pub fnc main(): int { return 42; }",
        "import core.num as math; pub fnc main(): int { return math.not_exported(42); }",
    ] {
        let mut loads = 0;
        assert!(compile_with_imports(source, Path::new("main.atto"), |_| {
            loads += 1; Ok(String::new())
        }).is_err(), "{source}");
        assert_eq!(loads, 0, "{source}");
    }
}

#[test]
fn builtin_header_parser_matches_all_six_binding_sets() {
    for (declaration, bindings) in [
        ("import namespace num from core.num;", vec!["num", "core.num"]),
        ("import num from core.num;", vec!["num", "core.num"]),
        ("import core.num from core.num;", vec!["core.num"]),
        ("import core.num as num;", vec!["num"]),
        ("import core.num as core.num;", vec!["core.num"]),
        ("import core.num;", vec!["num", "core.num"]),
        ("import namespace core.num;", vec!["core.num"]),
        ("import namespace num as arithmetic from 'core.num';", vec!["arithmetic"]),
        ("import namespace core.num as custom.math from \"core.num\";", vec!["custom.math"]),
    ] {
        assert_eq!(parse_v2(declaration).imports, bindings, "{declaration}");
    }
    // A pkg domain remains syntactically admissible, never builtin-trusted.
    assert_eq!(parse_v2("import num from pkg 'core.num';").imports, ["num"]);
    assert_eq!(parse_v2("import const PI from 'core.math';").const_imports, ["PI"]);
    for src in [
        "import core.num; import core.num as num;",
        "import core.num as root; import core.cmp as root.child;",
        "import core.num as core.num; pub fnc core(): int { return 1; }",
    ] {
        reject_v2(src, "E_DUPLICATE");
    }
}

#[test]
fn namespace_header_order_cannot_be_laundered_by_builtin_expansion() {
    for src in [
        "import core.num; use namespace app; pub fnc main(): int { return 0; }",
        "import num from core.num; use default namespace; pub fnc main(): int { return 0; }",
        "import module num from 'core/num'; use namespace app; pub fnc main(): int { return 0; }",
        "import core.num; use default namespace(1); pub fnc main(): int { return 0; }",
    ] {
        assert_eq!(compile(src).unwrap_err().code, "E_SYNTAX", "{src}");
        assert_eq!(atto_compiler::compile_with_path(src, Path::new("app/main.atto")).unwrap_err().code, "E_SYNTAX");
        let mut loads = 0;
        assert_eq!(compile_with_imports(src, Path::new("app/main.atto"), |_| {
            loads += 1; Ok(String::new())
        }).unwrap_err().code, "E_SYNTAX");
        assert_eq!(loads, 0);
    }
    assert_eq!(compile("use namespace app; import core.num; use namespace other; pub fnc main(): int { return 0; }").unwrap_err().code, "E_DUPLICATE");
    assert!(compile("use namespace app; import core.num; pub fnc main(): int { return core.num.clamp(42, 0, 100); }").is_ok());
}

#[test]
fn builtin_from_aliases_do_not_leak_identity_or_ignore_selected_name() {
    for src in [
        "import namespace num as math from core.num; pub fnc main(): int { return math.clamp(42, 0, 100); }",
        "import namespace core.num as custom.math from 'core.num'; pub fnc main(): int { return custom.math.clamp(42, 0, 100); }",
    ] {
        let wasm = compile(src).unwrap();
        assert_eq!(&wasm[..4], b"\0asm");
        assert_eq!(atto_compiler::compile_with_path(src, Path::new("app/main.atto")).unwrap(), wasm);
        let mut loads = 0;
        assert_eq!(compile_with_imports(src, Path::new("app/main.atto"), |_| {
            loads += 1; Ok(String::new())
        }).unwrap(), wasm);
        assert_eq!(loads, 0);
    }
    for src in [
        "import namespace unrelated as math from core.num;",
        "import namespace num as from from core.num;",
        "import namespace num as __atto_forged from core.num;",
        "import core.num as enum;",
        "import core.num as pkg;",
        "import core.num as math; pub fnc main(): int { return core.num.clamp(42, 0, 100); }",
        "import namespace num as math from core.num; pub fnc main(): int { return num.clamp(42, 0, 100); }",
    ] {
        assert!(compile(src).is_err(), "{src}");
    }
}

#[test]
fn multiline_alias_rewrite_preserves_lines_and_unicode_boundary() {
    let source = "import core.num as 数学.函数;\npub fnc main(): int {\n    return 数学 /* café\n    comment */ . 函数.clamp(42, 0, 100);\n}\n";
    let lowered = atto_compiler::library::expand(source).unwrap();
    let before_library = lowered.split("define module num as").next().unwrap();
    assert_eq!(before_library.lines().position(|line| line.contains(".clamp")), Some(3));
    assert!(compile(source).is_ok());
}

#[test]
fn builtin_bindings_budget_applies_to_repeated_aliases_of_one_unit() {
    let imports = (0..257).map(|i| format!("import core.num as alias{i};")).collect::<Vec<_>>().join("\n");
    let source = format!("{imports}\npub fnc main(): int {{ return 0; }}");
    assert_eq!(compile(&source).unwrap_err().code, "E_SOURCE_LIMIT");
    reject_v2(&imports, "E_SOURCE_LIMIT");
}
