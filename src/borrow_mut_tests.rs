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
