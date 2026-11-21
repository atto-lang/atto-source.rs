//! Positive and adversarial source-compiler coverage for file-local modules.
//! Actual compiled native/wasm32 guest execution and exact website stdout are
//! enforced separately by playground-preview-audit.mjs.
use atto_compiler::compile;

fn ok(src: &str) {
    let wasm = compile(src).unwrap_or_else(|e| panic!("compiler rejected {src}: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(compile(src).unwrap(), wasm, "deterministic Wasm");
}
fn code(src: &str) -> String {
    let e = compile(src).expect_err("unsupported/unsafe source must not produce Wasm");
    assert!(e.pos.line > 0 && e.pos.col > 0);
    e.code.into()
}

#[test]
fn exact_website_module_preview_is_a_real_guest() {
    ok(include_str!("playground_previews/modules.atto"));
}

#[test]
fn private_member_can_be_called_only_inside_own_module() {
    ok(r#"
        define module Counter as
            fnc increment(int x): int { return x + 1; }
            pub fnc answer(int x): int { return Counter.increment(x); }
        end
        pub fnc main(): int { return Counter.answer(41); }
    "#);
    assert_eq!(code(r#"
        define module Counter as
            fnc increment(int x): int { return x + 1; }
        end
        pub fnc main(): int { return Counter.increment(41); }
    "#), "E_PRIVATE");
}

#[test]
fn names_are_distinct_and_qualified_calls_are_resolved_at_compile_time() {
    ok(r#"
        define module A_B as pub fnc c(): int { return 10; } end
        define module A as pub fnc B_c(): int { return 32; } end
        pub fnc main(): int { return A_B.c() + A.B_c(); }
    "#);
    ok(r#"
        pub fnc main(): int { return Add.double(21); }
        define module Add as pub fnc double(int v): int { return v * 2; } end
    "#);
}

#[test]
fn forged_identifiers_duplicate_members_and_module_values_fail_closed() {
    for (source, expected) in [
        (r#"
            define module Math as pub fnc f(): int { return 1; } end
            define module Math as pub fnc g(): int { return 2; } end
            pub fnc main(): int { return 0; }
        "#, "E_DUPLICATE"),
        (r#"
            define module Math as
                pub fnc f(): int { return 1; }
                pub fnc f(int v): int { return v; }
            end
            pub fnc main(): int { return 0; }
        "#, "E_DUPLICATE"),
        (r#"
            define module Math as pub fnc f(): int { return 1; } end
            pub fnc main(): int { return Math.missing(); }
        "#, "E_UNBOUND"),
        (r#"
            define module Math as pub fnc f(): int { return 1; } end
            pub fnc main(): int { val Math = 42; return Math; }
        "#, "E_UNSUPPORTED"),
        (r#"
            define module Math as pub fnc f(): int { return 1; } end
            pub fnc main(): int { val captured = Math; return 0; }
        "#, "E_UNSUPPORTED"),
        (r#"
            define module Math as pub fnc f(): int { return 1; } end
            fnc __atto_internal_module_4_Math_1_f(): int { return 99; }
            pub fnc main(): int { return Math.f(); }
        "#, "E_RESERVED"),
        (r#"
            define module Math as pub fnc f(): int { return 1; } end
            pub fnc main(): int { return (Math.f); }
        "#, "E_UNSUPPORTED"),
        (r#"
            define module Math as int state: 10; end
            pub fnc main(): int { return 0; }
        "#, "E_UNSUPPORTED"),
    ] {
        assert_eq!(code(source), expected, "{source}");
    }
    ok("pub fnc main(): int { return 42; }");
}

#[test]
fn module_is_not_a_cross_file_import_or_linker() {
    assert!(compile(r#"
        import module External from "./external.atto";
        pub fnc main(): int { return External.answer(); }
    "#).is_err());
}
