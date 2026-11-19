//! Exact compiler integration for the opt-in `borrow-v1` source profile.
//! This is NOT general NLL borrowed-return or method/actor/async support.
use atto_compiler::{compile, compile_with_mode, OwnershipMode};

const HEADER: &str = r#"
    define class Person as pub let int age = 21; end
    fnc inspect(Person p): int { return p.age; }
"#;

fn borrow_v1(source: &str) -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_with_mode(source, OwnershipMode::BorrowV1)
}

#[test]
fn bare_formals_borrow_in_versioned_mode_only() {
    let src = format!(r#"{HEADER}
        pub fnc main(): int {{
            let person = new Person();
            let a = inspect(person);
            let b = inspect(person);
            return a + b;
        }}
    "#);
    assert_eq!(compile(&src).unwrap_err().code, "E_MOVED",
        "historical default must not silently change");
    let guest = borrow_v1(&src).expect("versioned bare Person formal is a loan");
    assert!(guest.starts_with(b"\0asm"));
}

#[test]
fn shorthand_and_explicit_borrow_have_identical_guest_bytes() {
    let implicit = format!(r#"{HEADER}
        pub fnc main(): int {{
            let p = new Person();
            return inspect(p);
        }}
    "#);
    let explicit = format!(r#"{HEADER}
        pub fnc main(): int {{
            let p = new Person();
            return inspect(borrow! p);
        }}
    "#);
    assert_eq!(borrow_v1(&implicit).unwrap(), borrow_v1(&explicit).unwrap());
}

#[test]
fn versioned_mode_preserves_scalar_value_abi() {
    let source = "fnc square(int x): int { return x * x; } pub fnc main(): int { return square(6) + 6; }";
    assert_eq!(compile(source).unwrap(), borrow_v1(source).unwrap());
}

#[test]
fn borrowed_formal_never_gains_ownership_after_last_use() {
    let source = r#"
        define class Person as pub let int age = 21; end
        fnc steal(Person p): Person { return p; }
        pub fnc main(): int {
            let person = new Person();
            let stolen = steal(person);
            return stolen.age;
        }
    "#;
    assert_eq!(borrow_v1(source).unwrap_err().code, "E_BORROW");
}

#[test]
fn unknown_or_provenance_unsafe_calls_fail_before_guest_emission() {
    let cases = [
        // A constructed temporary has no stable caller owner place.
        (format!(r#"{HEADER}
            fnc make(): Person {{ return new Person(); }}
            pub fnc main(): int {{ return inspect(make()); }}
        "#), "E_BORROW"),
        // Explicit ownership transfer is not a read loan.
        (format!(r#"{HEADER}
            pub fnc main(): int {{
                let p = new Person();
                return inspect(take! p);
            }}
        "#), "E_BORROW"),
        // A callee cannot mutate a borrowed ordinary class formal.
        (r#"
            define class Person as pub let int age = 21; end
            fnc change(Person p): void { p.age = 42; }
            pub fnc main(): int {
                let p = new Person();
                change(p);
                return 42;
            }
        "#.to_string(), "E_BORROW"),
        // Unproven returned borrow cannot be erased to an owned Person.
        (r#"
            define class Person as pub let int age = 21; end
            fnc view(Person p): Borrow<Person> { return p; }
            pub fnc main(): int { return 42; }
        "#.to_string(), "E_UNSUPPORTED_TYPE"),
        // Existing method parameter ABI has not been migrated.
        (r#"
            define class Person as pub let int age = 21; end
            define class Reader as
                pub inspect(Person p): int { return p.age; }
            end
            pub fnc main(): int { return 42; }
        "#.to_string(), "E_UNSUPPORTED_TYPE"),
        // Neither a checker error nor an explicit borrow can make an
        // unresolvable source type into a verified class owner.
        (r#"
            fnc inspect(Unknown p): int { return 1; }
            pub fnc main(): int { return 42; }
        "#.to_string(), "E_UNKNOWN_TYPE"),
    ];
    for (source, expected) in cases {
        let err = borrow_v1(&source).expect_err(
            "unsafe borrowing path must reject before emitting a guest");
        assert_eq!(err.code, expected, "{source}");
    }
}


#[test]
fn compiler_synthesized_resolved_class_formal_also_borrows() {
    // Programmatic typed-AST entry points must not bypass borrow-v1 just
    // because the frontend already resolved Person -> Type::Class(0).
    use atto_compiler::{ast::Type, check, lexer, parser::Parser};
    let source = r#"
        define class Person as pub let int age = 21; end
        fnc inspect(Person p): int { return p.age; }
        pub fnc main(): int {
            let p = new Person();
            return inspect(p) + inspect(p);
        }
    "#;
    let mut program = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    assert_eq!(program.funcs[0].name, "inspect");
    program.funcs[0].params[0].ty = Type::Class(0);
    let typed = check::check_borrow_v1(&program).expect("resolved class formal stays borrowed");
    // The typed module also contains synthesized class constructor entries;
    // counting exactly two functions would incorrectly reject valid lowering.
    assert_eq!(typed.funcs[0].name, "inspect");
}

#[test]
fn forged_class_index_and_unproved_mutable_formal_fail_closed() {
    use atto_compiler::{ast::Type, check, lexer, parser::Parser};
    let src = r#"
        define class Person as pub let int age = 21; end
        fnc inspect(Person p): int { return p.age; }
        pub fnc main(): int { return 42; }
    "#;
    let mut p = Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
    p.funcs[0].params[0].ty = Type::Class(1024);
    assert_eq!(check::check_borrow_v1(&p).err().unwrap().code, "E_UNKNOWN_TYPE");

    let src = r#"
        define class Person as pub let int age = 21; end
        fnc inspect(Person mut p): int { return p.age; }
        pub fnc main(): int { return 42; }
    "#;
    assert_eq!(borrow_v1(src).unwrap_err().code, "E_UNSUPPORTED_TYPE");
}


#[test]
fn stale_programmatic_ast_type_indices_are_diagnostics_not_panics() {
    // The public opt-in checker can receive an AST assembled by another
    // compiler stage. No forged Named/BorrowNamed/BorrowClass index may
    // panic via type_names[i] or classes[i], or enter unchecked codegen.
    use atto_compiler::{ast::Type, check, lexer, parser::Parser};
    let source = r#"
        define class Person as pub let int age = 21; end
        fnc inspect(Person p): int { return p.age; }
        pub fnc main(): int { return 42; }
    "#;
    for ty in [
        Type::Named(u32::MAX),
        Type::BorrowNamed(u32::MAX),
        Type::BorrowClass(u32::MAX),
        Type::Class(u32::MAX),
        Type::NamedRef(u32::MAX),
        Type::ActorRef(u32::MAX),
    ] {
        let mut p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
        p.funcs[0].params[0].ty = ty;
        let err = check::check_borrow_v1(&p).expect_err(
            "forged type index must fail before any unchecked array indexing");
        assert_eq!(err.code, "E_UNKNOWN_TYPE", "{ty:?}");
    }
    let mut p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    p.funcs[0].ret = Type::BorrowClass(u32::MAX);
    assert_eq!(check::check_borrow_v1(&p).unwrap_err().code, "E_UNKNOWN_TYPE");
    let mut p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    p.classes[0].fields[0].ty = Type::Named(u32::MAX);
    assert_eq!(check::check_borrow_v1(&p).unwrap_err().code, "E_UNKNOWN_TYPE");
}

#[test]
fn pre_resolved_borrowed_formal_cannot_become_an_owned_return() {
    // Even without the textual Borrow<...> spelling, the already-resolved
    // programmatic AST must carry permanent nonowning parameter provenance.
    use atto_compiler::{ast::Type, check, lexer, parser::Parser};
    let source = r#"
        define class Person as pub let int age = 21; end
        fnc steal(Person p): Person { return p; }
        pub fnc main(): int { return 42; }
    "#;
    for formal_ty in [Type::Class(0), Type::BorrowClass(0)] {
        let mut p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
        p.funcs[0].params[0].ty = formal_ty;
        let err = check::check_borrow_v1(&p).expect_err(
            "previously resolved formals are still not owners");
        assert_eq!(err.code, "E_BORROW", "{formal_ty:?}");
    }
}

#[test]
fn pre_resolved_borrowed_return_and_generator_are_not_lowered_as_owners() {
    use atto_compiler::{ast::Type, check, lexer, parser::Parser};
    let source = r#"
        define class Person as pub let int age = 21; end
        fnc view(Person p): Person { return p; }
        pub fnc main(): int { return 42; }
    "#;
    let mut p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    p.funcs[0].ret = Type::BorrowClass(0);
    assert_eq!(check::check_borrow_v1(&p).unwrap_err().code, "E_UNSUPPORTED_TYPE");

    let mut p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    p.funcs[0].params[0].ty = Type::BorrowClass(0);
    p.funcs[0].is_generator = true;
    assert_eq!(check::check_borrow_v1(&p).unwrap_err().code, "E_UNSUPPORTED_TYPE");
}


#[test]
fn programmatic_receive_payload_and_target_interface_fail_closed() {
    // Neither unsupported actor-interface signatures nor forged ActorMail<T>
    // payload metadata may bypass the opt-in source admission boundary.
    use atto_compiler::{ast::{ActorDecl, ActorInterface, Receive, Type},
        check, diag::Pos, lexer, parser::Parser};
    let source = "pub fnc main(): int { return 42; }";
    let base = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let pos = Pos { line: 1, col: 1 };
    for invalid_type in [
        Type::Named(u32::MAX), Type::BorrowNamed(u32::MAX),
        Type::BorrowClass(u32::MAX), Type::Class(u32::MAX),
    ] {
        let mut p = base.clone();
        p.actors.push(ActorDecl {
            name: "Worker".to_string(),
            pos,
            fields: Vec::new(),
            methods: Vec::new(),
            receive: Some(Receive {
                pos, mail: "mail".to_string(), payload: invalid_type,
                body: Vec::new()
            }),
        });
        let err = check::check_borrow_v1(&p).expect_err(
            "forged actor receive type must fail before mailbox codegen");
        assert_eq!(err.code, "E_UNKNOWN_TYPE", "{invalid_type:?}");
    }
    let mut p = base;
    p.actor_interface = Some(ActorInterface {
        namespace: None, forwards: Vec::new(), methods: Vec::new()
    });
    assert_eq!(check::check_borrow_v1(&p).unwrap_err().code, "E_UNSUPPORTED_TYPE");
}

#[test]
fn public_borrow_checker_respects_parser_cardinality_limits() {
    // Synthetic AST clients are not allowed to bypass the source parser's
    // bounded declaration/parameter/member dimensions.
    use atto_compiler::{check, lexer, parser::Parser};
    let source = r#"
        define class Parcel as pub let int value = 21; end
        fnc inspect(Parcel p): int { return p.value; }
        pub fnc main(): int { return 42; }
    "#;
    let base = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let mut oversized = base.clone();
    oversized.funcs[0].params = vec![
        oversized.funcs[0].params[0].clone(); atto_compiler::parser::MAX_PARAMS + 1
    ];
    assert_eq!(check::check_borrow_v1(&oversized).unwrap_err().code,
        "E_SOURCE_LIMIT", "too many formal parameters");
    let mut oversized = base.clone();
    oversized.funcs = vec![oversized.funcs[0].clone(); atto_compiler::parser::MAX_FUNCS + 1];
    assert_eq!(check::check_borrow_v1(&oversized).unwrap_err().code,
        "E_SOURCE_LIMIT", "too many functions");
    let mut oversized = base.clone();
    oversized.classes[0].fields = vec![
        oversized.classes[0].fields[0].clone(); atto_compiler::parser::MAX_MEMBERS + 1
    ];
    assert_eq!(check::check_borrow_v1(&oversized).unwrap_err().code,
        "E_SOURCE_LIMIT", "too many class fields");
    let mut oversized = base;
    let extra = oversized.classes[0].clone();
    oversized.classes = vec![extra; atto_compiler::parser::MAX_FUNCS + 1];
    assert_eq!(check::check_borrow_v1(&oversized).unwrap_err().code,
        "E_SOURCE_LIMIT", "too many classes");
}


#[test]
fn explicit_consuming_class_formal_is_owned_only_in_versioned_mode() {
    let source = r#"
        define class Person as pub let int age = 21; end
        fnc move_owned(take! Person p): Person { return p; }
        pub fnc main(): int {
            let p = new Person();
            let recovered = move_owned(take! p);
            return recovered.age + 21;
        }
    "#;
    assert_eq!(compile(source).unwrap_err().code, "E_UNSUPPORTED_TYPE",
        "legacy mode must never silently admit a new consuming-formal ABI");
    assert!(borrow_v1(source).unwrap().starts_with(b"\0asm"));
}

#[test]
fn consuming_formal_also_accepts_named_type_form_and_preserves_owner() {
    let source = r#"
        define class Person as pub let int age = 42; end
        fnc read_once(p: take! Person): int { return p.age; }
        pub fnc main(): int {
            let p = new Person();
            return read_once(take! p);
        }
    "#;
    assert!(borrow_v1(source).unwrap().starts_with(b"\0asm"));
}

#[test]
fn consuming_formal_requires_explicit_live_owner_transfer() {
    const HEADER: &str = r#"
        define class Person as pub let int age = 21; end
        fnc move_owned(take! Person p): Person { return p; }
    "#;
    let negatives = [
        (format!("{HEADER} pub fnc main(): int {{ let p = new Person(); let q = move_owned(p); return q.age; }}"),
         "E_OWNERSHIP"),
        (format!("{HEADER} pub fnc main(): int {{ let p = new Person(); let q = move_owned(borrow! p); return q.age; }}"),
         "E_OWNERSHIP"),
        (format!("{HEADER} fnc make(): Person {{ return new Person(); }} pub fnc main(): int {{ let q = move_owned(take! make()); return q.age; }}"),
         "E_OWNERSHIP"),
        (format!("{HEADER} pub fnc main(): int {{ let p = new Person(); let q = move_owned(take! p); return p.age; }}"),
         "E_MOVED"),
        (format!("{HEADER} pub fnc main(): int {{ let p = new Person(); let q = move_owned(take! p); let r = move_owned(take! p); return q.age; }}"),
         "E_MOVED"),
    ];
    for (source, expected) in negatives {
        assert_eq!(borrow_v1(&source).unwrap_err().code, expected, "{source}");
    }
}

#[test]
fn no_mutable_or_scalar_take_formals_without_proven_owner_domain() {
    for invalid in [
        "fnc unsupported(take! int a): int { return a; } pub fnc main(): int { return 42; }",
        "define class Person as pub let int age = 21; end fnc unsupported(take! Person mut p): int { return p.age; } pub fnc main(): int { return 42; }",
        "define class Person as pub let int age = 21; end fnc unsupported(take! Borrow<Person> p): int { return p.age; } pub fnc main(): int { return 42; }",
    ] {
        assert_eq!(borrow_v1(invalid).unwrap_err().code, "E_UNSUPPORTED_TYPE", "{invalid}");
    }
}
