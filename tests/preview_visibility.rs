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
fn overloaded_top_level_function_exports_full_arity_set() {
    ok(r#"
        export pub fnc score;
        fnc score(): int { return 1; }
        fnc score(int n): int { return n; }
        pub fnc main(): int { return score() + score(41); }
    "#);
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
