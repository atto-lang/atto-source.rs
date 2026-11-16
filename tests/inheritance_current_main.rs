//! Rebase admission tests for the bounded trait/interface/diamond compiler
//! subset. Exact website previews remain NOT certified until native and wasm32
//! compilers both execute the guest with byte and stdout/return equivalence.
//! This file intentionally does not change the 14/42 executed-preview ledger.
use atto_compiler::compile;

fn compile_guest(source: &str) {
    let wasm = compile(source).unwrap_or_else(|e| panic!("source failed: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "must emit real guest Wasm");
    assert_eq!(wasm, compile(source).unwrap(), "Wasm emission must be deterministic");
}

fn rejects(source: &str, expected: &str) {
    let error = compile(source).expect_err("invalid source must not emit guest Wasm");
    assert_eq!(error.code, expected, "{error}");
}

#[test]
fn exact_current_website_traits_interfaces_preview_compiles() {
    compile_guest(include_str!("playground_previews/traits-interfaces.atto"));
}

#[test]
fn exact_current_website_diamond_preview_compiles() {
    compile_guest(include_str!("playground_previews/diamond-inheritance.atto"));
}

#[test]
fn ambiguity_never_chooses_a_parent_by_order() {
    rejects(r#"
        define class Root as pub describe(): int { return 0; } end
        define class Left extends Root as pub override describe(): int { return 1; } end
        define class Right extends Root as pub override describe(): int { return 2; } end
        define class Broken extends Left, Right as end
        pub fnc main(): int { return 0; }
    "#, "E_AMBIGUOUS_MEMBER");
}

#[test]
fn override_is_explicit_and_signature_checked() {
    rejects(r#"
        define class Root as pub value(): int { return 0; } end
        define class Child extends Root as pub value(): int { return 1; } end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE_REQUIRED");
    rejects(r#"
        define class Root as pub value(): int { return 0; } end
        define class Child extends Root as pub override value(): bool { return true; } end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE_SIGNATURE");
}

#[test]
fn contract_and_ancestry_fail_closed() {
    rejects(r#"
        define interface Named as name(): string; end
        define class Missing implements Named as end
        pub fnc main(): int { return 0; }
    "#, "E_INTERFACE");
    rejects(r#"
        define class Left extends Right as end
        define class Right extends Left as end
        pub fnc main(): int { return 0; }
    "#, "E_INHERITANCE_CYCLE");
}

#[test]
fn conflicting_trait_defaults_require_explicit_resolution() {
    rejects(r#"
        define trait A as pub greet(): int { return 1; } end
        define trait B as pub greet(): int { return 2; } end
        define class Both implements A, B as end
        pub fnc main(): int { return 0; }
    "#, "E_AMBIGUOUS_MEMBER");
}
