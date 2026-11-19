//! Compiler-level admission for actor/struct required interface and trait methods.
//! Admitted contracts still fail E_UNSUPPORTED until independently verified
//! actor mailbox / struct value ABI is available. No guest execution claimed.
use atto_compiler::compile;

fn rejects(src: &str, wanted: &str) {
    let d = compile(src).expect_err("unproved or incomplete contract emitted Wasm");
    assert_eq!(d.code, wanted, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0, "{d}");
}

#[test]
fn actor_missing_required_interface_method() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}
#[test]
fn struct_missing_required_interface_method() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define struct S implements IPing as value: int; end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}
#[test]
fn required_abstract_trait_method_is_not_a_default_implementation() {
    for kind in ["actor", "struct"] {
        rejects(&format!(r#"
            define trait Required as ping(): int; end
            define {kind} Missing implements Required as end
            pub fnc main(): int {{ return 0; }}
        "#), "E_IMPL_MISSING");
    }
}
#[test]
fn actor_valid_inline_receiver_is_validated_but_not_guest_lowered() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as
            pub ping(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}
#[test]
fn actor_and_struct_out_of_line_mtd_satisfy_contract_but_stay_fail_closed() {
    for kind in ["actor", "struct"] {
        rejects(&format!(r#"
            define interface IPing as ping(): int; end
            declare {kind} Item implements IPing;
            define {kind} Item implements IPing as end
            pub mtd (Item self) ping(): int {{ return 42; }}
            pub fnc main(): int {{ return 0; }}
        "#), "E_UNSUPPORTED");
    }
}
#[test]
fn wrong_return_type_is_not_silently_accepted() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as
            pub ping(): bool { return true; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_SIGNATURE");
}
#[test]
fn private_out_of_line_method_does_not_satisfy_public_contract() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define struct S implements IPing as end
        mtd (S self) ping(): int { return 42; }
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_VISIBILITY");
}
#[test]
fn wrong_owner_mtd_does_not_satisfy_named_contract() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as end
        define actor Other as end
        pub mtd (Other self) ping(): int { return 42; }
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}
#[test]
fn unresolved_contract_and_wrong_nominal_kind_are_distinct() {
    rejects(r#"
        define actor A implements Unknown as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_UNRESOLVED");
    rejects(r#"
        define class Ordinary as end
        define struct S implements Ordinary as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_KIND");
}
#[test]
fn incompatible_forward_definition_is_rejected() {
    rejects(r#"
        define interface IPing as ping(): int; end
        declare actor A implements IPing;
        define actor A as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_SIGNATURE");
}
#[test]
fn duplicate_inline_and_out_of_line_receiver_is_rejected() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as
            pub ping(): int { return 42; }
        end
        pub mtd (A self) ping(): int { return 7; }
        pub fnc main(): int { return 0; }
    "#, "E_DUPLICATE");
}
#[test]
fn competing_defaults_cannot_resolve_by_trait_order() {
    rejects(r#"
        define trait A as pub ping(): int { return 1; } end
        define trait B as pub ping(): int { return 2; } end
        define struct S implements A, B as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_CONFLICT");
}
#[test]
fn unique_trait_default_may_satisfy_interface_but_cannot_emit_unverified_struct_abi() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define trait T as pub ping(): int { return 42; } end
        define struct S implements IPing, T as end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}

#[test]
fn forged_mtd_nested_inside_another_callable_cannot_satisfy_an_interface() {
    rejects(r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as end
        pub fnc outer(): int {
            pub mtd (A self) ping(): int { return 42; }
            return 0;
        }
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}
