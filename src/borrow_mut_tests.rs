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
    assert_eq!(main.locals.iter().filter(|ty| **ty == Type::Class(0)).count(), 1,
        "the caller needs only its original owned class local, not a second borrowed handle");
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

#[test]
fn borrow_getter_elision_preserves_nested_expression_lowering() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub let int value = 21; end
        fnc inspect(Parcel p): int { return p.value; }
        pub fnc main(): int {
            let parcel = new Parcel();
            return ACCESS;
        }
    "#;
    let calls = source.replace("ACCESS",
        "(inspect(parcel) + 1) * inspect(borrow! parcel)");
    let direct = source.replace("ACCESS",
        "(parcel.value + 1) * parcel.value");
    assert_eq!(
        crate::compile(&calls).expect("nested readonly calls compile"),
        crate::compile(&direct).expect("nested direct field reads compile"),
        "each proved readonly call must lower in expression order and without a runtime borrow wrapper"
    );
}

#[test]
fn borrow_getter_elision_never_rewrites_computed_getters() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub let int value = 21; end
        fnc inspect_plus(Parcel p): int { return p.value + 1; }
        pub fnc main(): int {
            let parcel = new Parcel();
            return ACCESS;
        }
    "#;
    let call = source.replace("ACCESS", "inspect_plus(parcel)");
    let direct = source.replace("ACCESS", "parcel.value + 1");
    assert_ne!(
        crate::compile(&call).expect("computed getter call compiles"),
        crate::compile(&direct).expect("direct field arithmetic compiles"),
        "a computed result is not the simple one-FieldGet typed-IR proof and must keep its call ABI"
    );
}

#[test]
fn readonly_getter_elision_reloads_after_mutation_and_distinguishes_instances() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub mut int value = 10; end
        fnc inspect(Parcel p): int { return p.value; }
        fnc update(mut Parcel p): void { p.value = 32; }
        pub fnc main(): int {
            let first = new Parcel();
            let second = new Parcel();
            val before = FIRST_READ;
            update(mut first);
            val after = SECOND_READ;
            val other = THIRD_READ;
            return before + after + other - 10;
        }
    "#;
    let calls = source
        .replace("FIRST_READ", "inspect(first)")
        .replace("SECOND_READ", "inspect(borrow! first)")
        .replace("THIRD_READ", "inspect(second)");
    let direct = source
        .replace("FIRST_READ", "first.value")
        .replace("SECOND_READ", "first.value")
        .replace("THIRD_READ", "second.value");
    assert_eq!(
        crate::compile(&calls).expect("readonly calls around exclusive update"),
        crate::compile(&direct).expect("direct reads around exclusive update"),
        "elision must reload the field after mutation and retain each receiver identity"
    );
}


#[test]
fn bae_distinguishes_nominal_layout_and_nonzero_field_offset() {
    let source = include_str!("../tests/fixtures/pass/borrow-getter-elision-v1-layout.atto");
    let direct = source.replace("read_left(left)", "left.value")
        .replace("read_right(borrow! right)", "right.value");
    let explicit = source.replace("read_left(left)", "read_left(borrow! left)")
        .replace("read_right(borrow! right)", "read_right(right)");
    let bytes = crate::compile(source).expect("typed getter calls");
    assert_eq!(bytes, crate::compile(&direct).expect("direct field reads"),
        "each nominal class's own field offset must be retained");
    assert_eq!(bytes, crate::compile(&explicit).expect("alternate loan spellings"));
}


#[test]
fn bae_two_field_checked_arithmetic_matches_direct_reads() {
    let source = include_str!("../tests/fixtures/pass/borrow-int-pair-v1.atto");
    for op in ["+", "-", "*"] {
        let input = source.replace("p.left - p.right", &format!("p.left {op} p.right"));
        let direct = input.replace("difference(parcel)", &format!("parcel.left {op} parcel.right"));
        let explicit = input.replace("difference(parcel)", "difference(borrow! parcel)");
        let bytes = crate::compile(&input).expect("readonly checked arithmetic");
        assert_eq!(bytes, crate::compile(&direct).expect("direct checked arithmetic"));
        assert_eq!(bytes, crate::compile(&explicit).expect("explicit readonly loan"));
    }
}

#[test]
fn bae_two_field_effectful_call_keeps_original_abi() {
    let source = include_str!("../tests/fixtures/pass/borrow-int-pair-v1.atto")
        .replace("return p.left - p.right;", "stdio.print(1); return p.left - p.right;");
    let direct = source.replace("difference(parcel)", "parcel.left - parcel.right");
    assert_ne!(crate::compile(&source).expect("effectful call"),
        crate::compile(&direct).expect("direct reads"));
}


#[test]
fn readonly_formal_forwarding_uses_original_handle_without_extra_locals() {
    let source = include_str!("../tests/fixtures/pass/borrow-formal-forward-v1.atto");
    let implicit = source.replace("calculate(borrow! p)", "calculate(p)");
    let bytes = crate::compile(source).expect("readonly formal forwarding");
    assert_eq!(bytes, crate::compile(&implicit).expect("implicit reborrow"));
    // Hold the enclosing function's own eligibility constant: a local
    // prevents it becoming a new two-field BAE proof in the direct variant.
    let non_inline = source.replace("return calculate(borrow! p);",
        "val result = calculate(borrow! p); return result;");
    let direct = non_inline.replace("calculate(borrow! p)", "p.left - p.right");
    assert_eq!(crate::compile(&non_inline).expect("nested read"),
        crate::compile(&direct).expect("direct checked arithmetic"),
        "eligible inner call must lower like direct field access");
    let program = typed(source);
    for name in ["forward", "outer"] {
        let f = program.funcs.iter().find(|f| f.name == name).unwrap();
        assert_eq!(f.param_ownership, [ParamOwnership::BorrowRead]);
        assert!(f.locals.is_empty(), "no phantom owned or borrowed local: {name}");
    }
}

#[test]
fn readonly_formal_forwarding_does_not_expand_authority() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub mut int value = 42; end
        fnc read(Parcel p): int { return p.value; }
        fnc write(mut Parcel p): int { p.value = 1; return p.value; }
        fnc consume(take! Parcel p): int { return p.value; }
        fnc make(Parcel p): Parcel { return new Parcel(); }
        fnc forward(Parcel p): int { BODY }
        pub fnc main(): int { let parcel = new Parcel(); return forward(parcel); }
    "#;
    for body in [
        "return write(mut p);",
        "return consume(take! p);",
        "val other = make(p); return other.value;",
        "val alias = borrow! p; return read(alias);",
    ] {
        let err = crate::compile(&source.replace("BODY", body)).expect_err(body);
        assert_eq!(err.code, "E_BORROW", "{body}");
    }
    let exclusive = source.replace("fnc forward(Parcel p)", "fnc forward(mut Parcel p)")
        .replace("forward(parcel)", "forward(mut parcel)")
        .replace("BODY", "return read(p);");
    assert_eq!(crate::compile(&exclusive).expect_err("exclusive formal not admitted").code, "E_BORROW");
}

#[test]
fn readonly_formal_forwarding_allows_two_shared_reads_but_not_lexical_aliases() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub let int value = 21; end
        fnc sum(Parcel a, Parcel b): int { return a.value + b.value; }
        fnc forward(Parcel p): int { return sum(p, borrow! p); }
        pub fnc main(): int { let parcel = new Parcel(); return forward(parcel); }
    "#;
    crate::compile(source).expect("two call-scoped readonly reborrows");
    let alias = source.replace("return forward(parcel);", "val alias = borrow! parcel; return forward(alias);");
    assert_eq!(crate::compile(&alias).expect_err("lexical alias remains unsupported").code, "E_BORROW");
}


#[test]
fn bae_two_readonly_arguments_preserve_nominal_layout_and_operand_order() {
    let source = include_str!("../tests/fixtures/pass/borrow-two-args-v1.atto");
    let direct = source.replace("difference(left, borrow! right)", "right.value - left.value");
    let alternate = source.replace("difference(left, borrow! right)", "difference(borrow! left, right)");
    let bytes = crate::compile(source).expect("two readonly arguments");
    assert_eq!(bytes, crate::compile(&direct).expect("direct reads"));
    assert_eq!(bytes, crate::compile(&alternate).expect("alternate borrow spellings"));
    let wrong = source.replace("difference(left, borrow! right)", "difference(right, borrow! left)");
    assert!(crate::compile(&wrong).is_err(), "nominal type mismatch");
    let effectful = source.replace("return b.value - a.value;", "stdio.print(1); return b.value - a.value;");
    let effectful_direct = effectful.replace("difference(left, borrow! right)", "right.value - left.value");
    assert_ne!(crate::compile(&effectful).expect("effectful call"),
        crate::compile(&effectful_direct).expect("direct reads"));
}

#[test]
fn bae_two_shared_arguments_may_alias_the_same_original_handle() {
    let source = r#"// atto-ownership: borrow-default-v1
        define class Parcel as pub let int value = 21; end
        fnc sum(Parcel a, Parcel b): int { return a.value + b.value; }
        pub fnc main(): int { let p = new Parcel(); return sum(p, borrow! p); }
    "#;
    let direct = source.replace("sum(p, borrow! p)", "p.value + p.value");
    assert_eq!(crate::compile(source).expect("same-owner readonly arguments"),
        crate::compile(&direct).expect("direct aliased reads"));
}


#[test]
fn bae_transitive_forward_chain_matches_direct_original_field_reads() {
    let source = include_str!("../tests/fixtures/pass/borrow-formal-forward-v1.atto");
    let direct = source.replace("return outer(parcel);", "return parcel.left - parcel.right;");
    assert_eq!(crate::compile(source).expect("forward chain"),
        crate::compile(&direct).expect("direct original fields"),
        "the complete bounded forwarding chain must elide at the caller");
}

#[test]
fn bae_transitive_effectful_terminal_preserves_call() {
    let source = include_str!("../tests/fixtures/pass/borrow-formal-forward-v1.atto")
        .replace("return p.left - p.right;", "stdio.print(1); return p.left - p.right;");
    let direct = source.replace("return outer(parcel);", "return parcel.left - parcel.right;");
    assert_ne!(crate::compile(&source).expect("effectful chain"),
        crate::compile(&direct).expect("direct fields"));
}
