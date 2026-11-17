//! Source-local sealed hierarchy admission. This checks the actual Rust->Wasm
//! compile path; it does NOT claim runtime subtype dispatch, cross-module
//! linking, or exhaustive sealed-pattern analysis.
use atto_compiler::compile;

fn ok(body: &str) {
    let wasm = compile(body).unwrap_or_else(|e| panic!("expected success: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(body).unwrap(), "Wasm must be deterministic");
}
fn err(body: &str, code: &str) {
    let d = compile(body).expect_err("invalid hierarchy emitted executable Wasm");
    assert_eq!(d.code, code, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0);
}
const MAIN: &str = "pub fnc main(): int { return 0; }";

#[test]
fn closed_branch_and_open_nonsealed_branch_are_admitted() {
    ok(r#"
        define sealed class Base permits Leaf, OpenBranch as
            pub value(): int { return 42; }
        end
        define final class Leaf extends Base as end
        define non-sealed class OpenBranch extends Base as end
        define class Further extends OpenBranch as end
        pub fnc main(): int {
            val x = new Leaf();
            val y = new Further();
            return x.value() + y.value();
        }
    "#);
}
#[test]
fn recursively_sealed_branch_must_close_its_own_direct_children() {
    ok(r#"
        define sealed class Root permits Branch as end
        define sealed class Branch extends Root permits Leaf as end
        define final class Leaf extends Branch as end
        pub fnc main(): int { return 42; }
    "#);
}
#[test]
fn ordinary_open_class_and_inheritance_still_work() {
    ok(r#"
        define class Parent as pub f(): int { return 42; } end
        define class Child extends Parent as end
        pub fnc main(): int {
            val c = new Child();
            return c.f();
        }
    "#);
    ok(MAIN);
}
#[test]
fn unauthorized_direct_child_fails_closed() {
    err(r#"
        define sealed class Base permits Good as end
        define final class Good extends Base as end
        define final class Evil extends Base as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
}
#[test]
fn direct_subclass_must_declare_closure_modifier() {
    err(r#"
        define sealed class Base permits Child as end
        define class Child extends Base as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
}
#[test]
fn permits_names_are_verified_and_never_open_world_placeholders() {
    err(r#"
        define sealed class Base permits Missing as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define sealed class Base permits Child as end
        define final class Child as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define sealed class Base permits Base as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define sealed class Base permits Child, Child as end
        define final class Child extends Base as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
}
#[test]
fn final_base_cannot_be_extended_at_any_depth() {
    err(r#"
        define final class Closed as end
        define class Child extends Closed as end
        pub fnc main(): int { return 0; }
    "#, "E_FINAL");
}
#[test]
fn nonsealed_must_be_a_direct_permitted_subclass() {
    err(r#"
        define non-sealed class Orphan as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define sealed class Root permits Middle as end
        define final class Middle extends Root as end
        define non-sealed class Descendant extends Middle as end
        pub fnc main(): int { return 0; }
    "#, "E_FINAL");
}
#[test]
fn indirect_descendants_of_nonsealed_are_not_restricted() {
    ok(r#"
        define sealed class Root permits Flexible as end
        define non-sealed class Flexible extends Root as end
        define class Open extends Flexible as end
        define class Anything extends Open as end
        pub fnc main(): int { return 0; }
    "#);
}
#[test]
fn no_permits_on_open_or_final_class() {
    err(r#"
        define class Open permits Child as end
        define class Child extends Open as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define final class Closed permits Child as end
        define class Child extends Closed as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
}
#[test]
fn sealed_requires_nonempty_explicit_list() {
    err(r#"
        define sealed class S as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
}
#[test]
fn modifiers_cannot_change_trait_or_interface_or_actor_kind() {
    err(r#"
        define sealed trait Bad permits C as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define final interface Bad as f(): int; end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define sealed actor Bad as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
}
#[test]
fn multiple_base_paths_cannot_bypass_final_or_sealed_rules() {
    err(r#"
        define sealed class A permits Allowed as end
        define class B as end
        define final class Allowed extends A as end
        define final class Bad extends B, A as end
        pub fnc main(): int { return 0; }
    "#, "E_SEALED");
    err(r#"
        define final class No as end
        define class Yes as end
        define class Child extends Yes, No as end
        pub fnc main(): int { return 0; }
    "#, "E_FINAL");
}
#[test]
fn inherited_method_abi_still_requires_explicit_correct_override() {
    err(r#"
        define sealed class Root permits Leaf as
            pub f(): int { return 42; }
        end
        define final class Leaf extends Root as
            pub f(): int { return 3; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_OVERRIDE_REQUIRED");
}
