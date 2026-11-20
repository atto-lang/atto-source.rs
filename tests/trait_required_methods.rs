//! Compiler/guest admission for required trait signatures in the existing
//! bounded class nominal-lowering profile. Not an actor/struct runtime claim.
use atto_compiler::compile;

fn accepts(src: &str) {
    let wasm = compile(src).expect("concrete class fulfils every trait requirement");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(src).unwrap(), "deterministic guest Wasm");
}
fn rejects(src: &str, code: &str) {
    let d = compile(src).expect_err("broken trait obligation must not emit Wasm");
    assert_eq!(d.code, code, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0);
}

#[test]
fn class_implements_abstract_trait_with_concrete_method() {
    accepts(r#"
        define trait Required as ping(): int; end
        define class C implements Required as
            pub ping(): int { return 42; }
        end
        pub fnc main(): int { return new C().ping(); }
    "#);
}
#[test]
fn inherited_implementation_satisfies_required_trait() {
    accepts(r#"
        define trait Required as ping(): int; end
        define class Base as pub ping(): int { return 42; } end
        define class Child extends Base implements Required as end
        pub fnc main(): int { return new Child().ping(); }
    "#);
}
#[test]
fn unique_trait_default_satisfies_another_abstract_trait() {
    accepts(r#"
        define trait Required as ping(): int; end
        define trait Default as pub ping(): int { return 42; } end
        define class C implements Required, Default as end
        pub fnc main(): int { return new C().ping(); }
    "#);
}
#[test]
fn missing_abstract_trait_member_fails_before_codegen() {
    rejects(r#"
        define trait Required as ping(): int; end
        define class Broken implements Required as end
        pub fnc main(): int { return 0; }
    "#, "E_INTERFACE");
}
#[test]
fn abstract_trait_member_must_be_public() {
    rejects(r#"
        define trait Required as ping(): int; end
        define class Broken implements Required as
            ping(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_PRIVATE");
}
#[test]
fn abstract_trait_member_must_have_same_signature() {
    rejects(r#"
        define trait Required as ping(): int; end
        define class Broken implements Required as
            pub ping(): bool { return true; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_SIGNATURE");
}
#[test]
fn competing_abstract_trait_requirements_are_not_silently_erased() {
    rejects(r#"
        define trait A as ping(): int; end
        define trait B as ping(): bool; end
        define class Broken implements A, B as
            pub ping(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_SIGNATURE");
}
#[test]
fn two_concrete_trait_defaults_remain_ambiguous() {
    rejects(r#"
        define trait A as pub ping(): int { return 1; } end
        define trait B as pub ping(): int { return 2; } end
        define class Broken implements A, B as end
        pub fnc main(): int { return 0; }
    "#, "E_AMBIGUOUS_MEMBER");
}
#[test]
fn unrelated_method_same_name_with_wrong_arity_cannot_satisfy_trait() {
    rejects(r#"
        define trait Required as ping(): int; end
        define class Broken implements Required as
            pub ping(int n): int { return n; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_SIGNATURE");
}
