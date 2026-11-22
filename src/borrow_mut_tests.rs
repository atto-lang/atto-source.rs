//! Direct typed-IR admission tests for versioned class formal authority.
//! Actual guest execution and the full fixture matrix run in carrier CI.
use crate::{ast::{ParamOwnership, Type}, check, lexer, parser::Parser};

fn typed(source: &str) -> check::TProgram {
    let mut program = Parser::new(lexer::lex(source).expect("lex"))
        .parse_program().expect("parse");
    super::prepare_borrow_default_v1(&mut program).expect("versioned profile admission");
    check::check(&program).expect("typed versioned program")
}

#[test]
fn every_formal_ownership_mode_survives_typed_ir_even_after_wasm_abi_erasure() {
    let source = r#"
        define class Parcel as pub mut int value = 0; end
        fnc inspect(Parcel p): int { return p.value; }
        fnc update(mut Parcel p): void { p.value = 42; }
        fnc consume(take! Parcel p): int { return p.value; }
        pub fnc main(): int {
            let parcel = new Parcel();
            val observed = inspect(parcel);
            update(mut parcel);
            return observed + parcel.value;
        }
    "#;
    let module = typed(source);
    for (name, mode) in [
        ("inspect", ParamOwnership::BorrowRead),
        ("update", ParamOwnership::BorrowWrite),
        ("consume", ParamOwnership::Take),
    ] {
        let f = module.funcs.iter().find(|f| f.name == name)
            .expect("named typed function");
        assert_eq!(f.param_ownership, [mode], "{name}: type authority lost");
        assert_eq!(f.params, [Type::Class(0)],
            "{name}: guest ABI must use the same class handle type");
    }
}

#[test]
fn explicit_and_ergonomic_mut_borrows_have_identical_guest_bytes() {
    let base = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub mut int value = 0; end
        fnc update(mut Parcel p): void { p.value = 42; }
        pub fnc main(): int {
            let parcel = new Parcel();
            CALL_PLACEHOLDER
            return parcel.value;
        }
    "#;
    let ergonomic = base.replace("CALL_PLACEHOLDER", "update(mut parcel);");
    let explicit = base.replace("CALL_PLACEHOLDER", "update(borrow! mut parcel);");
    let ergonomic_wasm = crate::compile(&ergonomic).expect("ergonomic mutable borrow");
    let explicit_wasm = crate::compile(&explicit).expect("explicit mutable borrow");
    assert_eq!(ergonomic_wasm, explicit_wasm,
        "mutable borrow syntax must not create a distinct runtime object or guest ABI");
}

#[test]
fn explicit_owned_formal_is_invalid_without_migration_profile() {
    let source = r#"
        define class Parcel as pub let int value = 42; end
        fnc consume(take! Parcel p): int { return p.value; }
        pub fnc main(): int { return 42; }
    "#;
    let err = crate::compile(source).expect_err("v1-only formal cannot silently become legacy");
    assert_eq!(err.code, "E_PROFILE");
}

#[test]
fn canonical_mut_formal_is_not_a_mutable_owned_argument_in_legacy_modules() {
    let source = r#"
        define class Parcel as pub mut int value = 42; end
        fnc update(mut Parcel p): void { p.value = 1; }
        pub fnc main(): int { return 42; }
    "#;
    let err = crate::compile(source).expect_err("mut borrowed formal requires opt-in");
    assert_eq!(err.code, "E_PROFILE");
}

#[test]
fn readonly_borrowed_getter_elides_call_without_copying_guest_handle() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub let int value = 42; end
        fnc inspect(Parcel p): int { return p.value; }
        pub fnc main(): int {
            let parcel = new Parcel();
            return ACCESS;
        }
    "#;
    let implicit = source.replace("ACCESS", "inspect(parcel)");
    let explicit = source.replace("ACCESS", "inspect(borrow! parcel)");
    let direct = source.replace("ACCESS", "parcel.value");

    let implicit_bytes = crate::compile(&implicit).expect("borrowed accessor compiles");
    assert_eq!(implicit_bytes, crate::compile(&explicit).expect("explicit borrow"));
    assert_eq!(implicit_bytes, crate::compile(&direct).expect("direct field read"),
        "verified readonly getter must emit exactly the same Wasm as direct access");

    let typed = typed(&implicit);
    let main = typed.funcs.iter().find(|f| f.name == "main").expect("main");
    assert!(main.locals.iter().all(|ty| *ty != Type::Class(0)),
        "a borrowed call must not allocate a second class-handle local");
}

#[test]
fn borrow_getter_elision_rejects_effects_and_mutable_formals() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub mut int value = 42; end
        fnc inspect_effect(Parcel p): int {
            stdio.print(1);
            return p.value;
        }
        fnc update(mut Parcel p): int {
            p.value = 41;
            return p.value;
        }
        pub fnc main(): int {
            let parcel = new Parcel();
            return ACCESS;
        }
    "#;
    let effect_call = source.replace("ACCESS", "inspect_effect(parcel)");
    let effect_direct = source.replace("ACCESS", "parcel.value");
    assert_ne!(crate::compile(&effect_call).expect("effectful call"),
        crate::compile(&effect_direct).expect("direct field"),
        "observable callee effects cannot be optimized away");
    let update_call = source.replace("ACCESS", "update(mut parcel)");
    assert_ne!(crate::compile(&update_call).expect("mutable call"),
        crate::compile(&effect_direct).expect("direct read"),
        "exclusive mutable callee may write; it must not be treated as readonly");
}
