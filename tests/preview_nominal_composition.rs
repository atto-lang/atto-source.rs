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

#[test]
fn single_inherited_method_cannot_be_silently_shadowed() {
    rejected(r#"
        define class Base as
            pub ping(): int { return 1; }
        end
        define class Child extends Base as
            pub ping(): int { return 2; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE_REQUIRED");
}

#[test]
fn child_override_must_preserve_parent_signature() {
    rejected(r#"
        define class Base as
            pub ping(int n): int { return n; }
        end
        define class Child extends Base as
            pub override ping(int n): bool { return true; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_SIGNATURE");
}

#[test]
fn explicit_diamond_override_must_match_all_parent_signatures() {
    rejected(r#"
        define class A as pub ping(): int { return 1; } end
        define class B as pub ping(): bool { return true; } end
        define class C extends A, B as
            pub override ping(): int { return 3; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_SIGNATURE");
}

#[test]
fn concrete_method_override_is_allowed_through_indirect_ancestor() {
    compiled(r#"
        define class Base as pub ping(): int { return 1; } end
        define class Middle extends Base as end
        define class Child extends Middle as
            pub override ping(): int { return 2; }
        end
        pub fnc main(): int {
            val x = new Child();
            return x.ping();
        }
    "#);
}

#[test]
fn inherited_public_override_cannot_become_private() {
    rejected(r#"
        define class Base as pub ping(): int { return 1; } end
        define class Child extends Base as
            private override ping(): int { return 2; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_PRIVATE");
}

#[test]
fn trait_and_interface_members_cannot_masquerade_as_class_overrides() {
    rejected(r#"
        define trait T as
            pub override ping(): int { return 2; }
        end
        define class C implements T as end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE");
    rejected(r#"
        define interface I as
            override ping(): int;
        end
        define class C implements I as end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE");
}

#[test]
fn duplicate_bases_and_contracts_are_rejected_before_composition() {
    rejected(r#"
        define class A as pub ping(): int { return 1; } end
        define class B extends A, A as end
        pub fnc main(): int { return 0; }
    "#, "E_DUPLICATE");
    rejected(r#"
        define trait T as pub ping(): int { return 1; } end
        define class C implements T, T as end
        pub fnc main(): int { return 0; }
    "#, "E_DUPLICATE");
}

#[test]
fn inherited_field_alias_v1_reports_precise_fail_closed_boundaries() {
    // A valid alias is recognized but MUST NOT be silently flattened into an
    // unrelated new field before the inherited physical layout ABI exists.
    compiled(r#"
        define class Dog as pub int age: 5; end
        define class Cat as pub int age: 10; end
        define class Hybrid extends Dog, Cat as
            use Dog.age as dog_age;
            use Cat.age as cat_age;
        end
        pub fnc main(): int {
            val hybrid = new Hybrid();
            return hybrid.dog_age + hybrid.cat_age;
        }
    "#);

    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as use Dog::age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_SYNTAX");

    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_TARGET");

    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as use Dog.age dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_SYNTAX");

    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as use Dog.age as dog_age end
        pub fnc main(): int { return 0; }
    "#, "E_SYNTAX");

    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as pub use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_SYNTAX");
}

#[test]
fn ordinary_class_method_named_use_is_not_a_field_alias() {
    compiled(r#"
        define class Base as pub speak(): int { return 2; } end
        define class Child extends Base as
            pub use(): int { return 5; }
        end
        pub fnc main(): int { return new Child().use(); }
    "#);
}

#[test]
fn exact_direct_scalar_parent_fields_compile_deterministic_wasm() {
    compiled(r#"
        define class Dog as pub int age: 5; end
        define class Cat as pub int age: 10; end
        define class Hybrid extends Dog, Cat as
            use Cat.age as cat_age;
            use Dog.age as dog_age;
        end
        pub fnc main(): int {
            val hybrid = new Hybrid();
            return hybrid.cat_age + hybrid.dog_age;
        }
    "#);
    compiled(r#"
        define class Enabled as pub bool ready: true; end
        define class Hybrid extends Enabled as use Enabled.ready as is_ready; end
        pub fnc main(): int {
            val item = new Hybrid();
            if item.is_ready then return 42; else return 0; fi
        }
    "#);
}
#[test]
fn scalar_parent_aliases_reject_wrong_origin_inaccessible_or_duplicate_storage() {
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Cat as pub int age: 10; end
        define class Hybrid extends Dog as use Cat.age as cat_age; end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_TARGET");
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as use Dog.missing as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_TARGET");
    rejected(r#"
        define class Dog as private int age: 5; end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_VISIBILITY");
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as
            use Dog.age as age_one;
            use Dog.age as age_two;
        end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_DUPLICATE");
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Cat as pub int age: 10; end
        define class Hybrid extends Dog, Cat as
            use Dog.age as same_age;
            use Cat.age as same_age;
        end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_DUPLICATE");
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as use Dog.age as age; end
        pub fnc main(): int { return 0; }
    "#, "E_ALIAS_DUPLICATE");
}
#[test]
fn stateful_parent_layouts_without_proof_remain_fail_closed() {
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return new Hybrid().age; }
    "#, "E_UNBOUND");
    rejected(r#"
        define class Dog as pub int age: 5; end
        define class Hybrid extends Dog as end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        define class Dog as pub int age: 5; pub speak(): int { return self.age; } end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        define class Dog as pub int age: 5; constructor() {} end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        define class Dog as pub int age: 5; pub int mood: 4; end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        define class Dog as pub int age: 1 + 4; end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    rejected(r#"
        define class Animal as pub int age: 7; end
        define class Dog extends Animal as end
        define class Hybrid extends Dog as use Dog.age as dog_age; end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}
