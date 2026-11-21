//! Narrow actor-local mtd runtime, using the existing mailbox-only actor backend.
use atto_compiler::compile;

const GOOD: &str = r#"
    define interface IDouble as double(int input): int; end
    declare actor Worker implements IDouble;
    define actor Worker implements IDouble as
        receive(ActorMail<int> mail): void {
            stdio.println(self.double(mail.value));
        }
    end
    pub mtd (Worker self) double(int n): int { return n * 2; }
    pub fnc main(): int { return 0; }
"#;
fn fail(src: &str, code: &str) {
    let d = compile(src).expect_err("invalid nominal actor must not emit Wasm");
    assert_eq!(d.code, code, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0);
}
#[test]
fn compiled_actor_local_receiver_is_deterministic_real_wasm() {
    let binary = compile(GOOD).expect("actor method should compile through existing actor emitter");
    assert!(binary.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(binary, compile(GOOD).unwrap());
    assert!(binary.windows(b"actor_receive".len()).any(|x| x == b"actor_receive"));
}
#[test]
fn actor_inline_required_method_is_compiled_after_contract_proof() {
    let src = r#"
        define interface I as ping(): int; end
        define actor A implements I as
            pub ping(): int { return 42; }
            receive(ActorMail<int> mail): void {
                stdio.println(self.ping());
            }
        end
        pub fnc main(): int { return 0; }
    "#;
    assert!(compile(src).unwrap().starts_with(b"\0asm"));
}
#[test]
fn declared_actor_contract_mismatch_stays_rejected() {
    fail(r#"
        define interface I as ping(): int; end
        declare actor A implements I;
        define actor A as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_SIGNATURE");
}
#[test]
fn missing_actor_contract_method_does_not_emit_guest() {
    fail(r#"
        define interface I as ping(): int; end
        define actor A implements I as end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}
#[test]
fn wrong_signature_fails_before_actor_dispatch_is_lowered() {
    fail(r#"
        define interface I as ping(): int; end
        define actor A implements I as end
        pub mtd (A self) ping(): bool { return true; }
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_SIGNATURE");
}
#[test]
fn wrong_nominal_receiver_owner_never_satisfies_actor_contract() {
    fail(r#"
        define interface I as ping(): int; end
        define actor A implements I as end
        define actor B as end
        pub mtd (B self) ping(): int { return 42; }
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}
#[test]
fn duplicate_inline_and_external_implementation_is_rejected() {
    fail(r#"
        define interface I as ping(): int; end
        define actor A implements I as
            pub ping(): int { return 42; }
        end
        pub mtd (A self) ping(): int { return 4; }
        pub fnc main(): int { return 0; }
    "#, "E_DUPLICATE");
}
#[test]
fn external_direct_actorref_method_call_is_still_illegal() {
    let source = r#"
        define interface IPing as ping(): int; end
        define actor A implements IPing as pub ping(): int { return 42; } end
        pub fnc main(): int {
            val worker_ref = spawn A();
            return worker_ref.ping();
        }
    "#;
    let d = compile(source).expect_err("external actorref call must not be executable");
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
}
#[test]
fn trait_default_actor_implementation_is_not_silently_dropped() {
    fail(r#"
        define trait Default as pub ping(): int { return 42; } end
        define actor A implements Default as end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}
#[test]
fn struct_value_receiver_remains_explicitly_unsupported() {
    fail(r#"
        define interface I as ping(): int; end
        define struct S implements I as
            pub ping(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}

#[test]
fn contract_lowering_never_downgrades_actor_owned_state_to_ordinary_fields() {
    fail(r#"
        define interface I as ping(): int; end
        define actor A implements I as
            let int total = 0;
            pub ping(): int { return self.total; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
    fail(r#"
        define interface I as ping(): int; end
        define actor A implements I as
            value: int = 0;
            pub ping(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}
