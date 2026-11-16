//! Checked source-local nominal composition used by exact website previews.
//! The actual wasm32 compiler, guest stdout and byte parity are verified by
//! playground-preview-audit.mjs. No dynamic inheritance/dispatch is implied.
use atto_compiler::compile;

fn compiled(source: &str) {
    let bytes = compile(source).unwrap_or_else(|d| panic!("expected guest Wasm: {d}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(source).unwrap(), "deterministic Wasm");
}
fn rejected(source: &str, wanted: &str) {
    let d = compile(source).expect_err("invalid nominal composition must emit no Wasm");
    assert_eq!(d.code, wanted, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0);
}
#[test]
fn exact_website_interface_trait_and_diamond_sources_execute_as_guest_candidates() {
    compiled(include_str!("playground_previews/traits-interfaces.atto"));
    compiled(include_str!("playground_previews/diamond-inheritance.atto"));
}
#[test]
fn ambiguous_multiple_concrete_implementations_require_explicit_override() {
    rejected(r#"
        define class Root as pub describe(): string { return "root"; } end
        define class Left extends Root as pub override describe(): string { return "left"; } end
        define class Right extends Root as pub override describe(): string { return "right"; } end
        define class Diamond extends Left, Right as end
        pub fnc main(): int { return 0; }
    "#, "E_AMBIGUOUS_MEMBER");
}
#[test]
fn interface_contracts_must_be_public_and_complete() {
    rejected(r#"
        define pub interface Named as name(): string; end
        define class Person implements Named as end
        pub fnc main(): int { return 0; }
    "#, "E_INTERFACE");
    rejected(r#"
        define pub interface Named as name(): string; end
        define class Person implements Named as
            name(): string { return "secret"; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_PRIVATE");
    rejected(r#"
        define pub interface Named as name(): string; end
        define class Person implements Named as
            pub name(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_SIGNATURE");
}
#[test]
fn cycles_and_inherited_fields_or_constructors_fail_closed() {
    rejected(r#"
        define class A extends B as end
        define class B extends A as end
        pub fnc main(): int { return 0; }
    "#, "E_CYCLE");
    rejected(r#"
        define class Parent as int secret: 10; end
        define class Child extends Parent as end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        define class Parent as constructor() {} end
        define class Child extends Parent as end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}
#[test]
fn unknown_and_duplicate_nominal_declarations_are_rejected() {
    rejected("define class C extends Missing as end pub fnc main(): int { return 0; }", "E_UNBOUND");
    rejected("define class C as end define pub interface C as f(): int; end pub fnc main(): int { return 0; }", "E_DUPLICATE");
    rejected(r#"
        define class Parent as end
        define class Child extends Parent as
            pub override invented(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE");
    compiled("pub fnc main(): int { return 42; }");
}
