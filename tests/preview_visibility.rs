//! Exact website visibility preview: real guest Wasm and fail-closed exports.
//! The output oracle and native/wasm32 compiler byte comparison are enforced
//! independently by the playground audit.
use atto_compiler::compile;
fn ok(src: &str) -> Vec<u8> {
    let wasm = compile(src).unwrap_or_else(|e| panic!("expected compiled guest: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(src).unwrap());
    wasm
}
fn err(src: &str, code: &str) {
    let e = compile(src).expect_err("invalid source must not emit Wasm");
    assert_eq!(e.code, code, "{e}");
    assert!(e.pos.line > 0 && e.pos.col > 0);
}
#[test]
fn exact_website_visibility_source_executes_and_has_deterministic_metadata() {
    let wasm = ok(include_str!("playground_previews/visibility-modifiers.atto"));
    assert!(wasm.windows(b"atto.visibility.profile.v1".len())
        .any(|w| w == b"atto.visibility.profile.v1"));
}
#[test]
fn forward_export_and_package_export_have_no_host_functions() {
    ok(r#"
        export pub Root;
        define class Root as pub answer(): int { return 42; } end
        export pub[pkg] class Internal;
        define class Internal as pub answer(): int { return 0; } end
        pub fnc main(): int { val x = new Root(); return x.answer(); }
    "#);
}
#[test]
fn duplicate_missing_and_wrong_kind_exports_reject_without_guest() {
    err("export pub Foo; pub fnc main(): int { return 0; }", "E_UNBOUND");
    err(r#"
        export pub Foo;
        export pub[pkg] Foo;
        define class Foo as end
        pub fnc main(): int { return 0; }
    "#, "E_DUPLICATE");
    err(r#"
        export pub fnc Foo;
        define class Foo as end
        pub fnc main(): int { return 0; }
    "#, "E_TYPE");
    err("export pub[guest] Foo; pub fnc main(): int { return 0; }", "E_SYNTAX");
}
#[test]
fn foreign_prototypes_never_grant_implicit_abi_capabilities() {
    err(r#"
        external fnc host_clock_ns(): int;
        pub fnc main(): int { return host_clock_ns(); }
    "#, "E_UNSUPPORTED");
    err(r#"
        external fnc host_clock_ns(): int;
        pub fnc main(): int { val alias = host_clock_ns; return 0; }
    "#, "E_UNSUPPORTED");
    err(r#"
        external fnc host_clock_ns(): int;
        external fnc host_clock_ns(): int;
        pub fnc main(): int { return 0; }
    "#, "E_DUPLICATE");
}
#[test]
fn protected_modifier_cannot_create_public_mutable_alias() {
    err("define class C as protected let int mutable: 42; end pub fnc main(): int { return 0; }", "E_UNSUPPORTED");
    ok("pub fnc main(): int { return 42; }");
}

#[test]
fn class_method_names_do_not_forge_file_level_exports() {
    err(r#"
        export pub fnc hidden;
        define class C as
            pub hidden(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_UNBOUND");
}

#[test]
fn overloaded_top_level_function_needs_nonambiguous_export_identity() {
    err(r#"
        export pub fnc score;
        fnc score(int n): int { return n; }
        fnc score(bool b): int { return 42; }
        pub fnc main(): int { return score(1); }
    "#, "E_DUPLICATE");
}

#[test]
fn foreign_prototype_cannot_satisfy_export_or_become_callable() {
    err(r#"
        export pub fnc host_clock_ns;
        external fnc host_clock_ns(): int;
        pub fnc main(): int { return 0; }
    "#, "E_UNBOUND");
}

#[test]
fn genuine_top_level_function_export_resolves_without_host_function_leak() {
    let wasm = ok(r#"
        export pub fnc get_value;
        fnc get_value(): int { return 42; }
        pub fnc main(): int { return get_value(); }
    "#);
    assert!(wasm.windows(b"atto.visibility.profile.v1".len())
        .any(|w| w == b"atto.visibility.profile.v1"));
}

#[test]
fn identical_exports_are_idempotent_in_both_orders() {
    use atto_compiler::{lexer::lex, visibility_lower::lower};
    for directives in [
        "export pub get_value; export pub fnc get_value;",
        "export pub fnc get_value; export pub get_value;",
        "export pub fnc get_value; export pub fnc get_value;",
        "export pub[pkg] fnc get_value; export pub[pkg] get_value;",
    ] {
        let src = format!("{directives} fnc get_value(): int {{ return 42; }} pub fnc main(): int {{ return get_value(); }}");
        let lowered = lower(lex(&src).unwrap()).unwrap();
        assert_eq!(lowered.exports.len(), 1);
        assert_eq!(lowered.exports[0].kind, "fnc");
        ok(&src);
    }
    err("export pub fnc get_value; export pub class get_value; fnc get_value(): int { return 42; } pub fnc main(): int { return 0; }", "E_TYPE");
}

#[test]
fn erased_internal_marker_cannot_grant_export_authority() {
    ok("internal fnc helper(): int { return 42; } pub fnc main(): int { return helper(); }");
    for directive in ["export pub fnc helper;", "export pub[pkg] helper;"] {
        for forward in [true, false] {
            let decl = "internal fnc helper(): int { return 42; }";
            let src = if forward { format!("{directive} {decl}") }
                else { format!("{decl} {directive}") };
            err(&format!("{src} pub fnc main(): int {{ return helper(); }}"), "E_VISIBILITY");
        }
    }
    for header in ["pub internal fnc", "internal pub fnc", "pub[pkg] internal fnc"] {
        err(&format!("{header} helper(): int {{ return 42; }} pub fnc main(): int {{ return 0; }}"), "E_VISIBILITY");
    }
    for header in ["pub external fnc", "pub[pkg] external fnc"] {
        err(&format!("{header} host_clock_ns(): int; pub fnc main(): int {{ return 0; }}"), "E_VISIBILITY");
    }
}

#[test]
fn nested_brace_export_does_not_promote_a_file_root_symbol() {
    for source in [
        "fnc helper(): int { return 42; } pub fnc main(): int { export pub fnc helper; return helper(); }",
        "fnc helper(): int { return 42; } define class C as pub value(): int { export pub fnc helper; return 42; } end pub fnc main(): int { return helper(); }",
    ] {
        err(source, "E_SCOPE");
    }
    ok("fnc helper(): int { return 42; } pub fnc main(): int { return helper(); } export pub fnc helper;");
}
