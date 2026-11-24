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
        "import ((module x)) from pkg '@foo/bar/baz';",
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
