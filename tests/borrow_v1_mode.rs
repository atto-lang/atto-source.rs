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
