//! Actual frontend -> typed-IR evidence for bounded class loans.
//! This is not a certified region verifier, drop proof, or cross-backend ABI.
use atto_compiler::{check::{self, TStmt}, lexer, parser::Parser};

fn checked(src: &str) -> check::TProgram {
    let program = Parser::new(lexer::lex(src).expect("lex"))
        .parse_program().expect("parse");
    check::check(&program).expect("typed source")
}

#[test]
fn issue_and_end_are_preserved_in_real_checked_ir() {
    let src = r#"
        define class Parcel as
            pub let int value = 40;
        end
        pub fnc main(): int {
            let parcel = new Parcel();
            val reader = borrow! parcel;
            stdio.println(reader.value);
            val writer = borrow! mut parcel;
            writer.value = 42;
            return parcel.value;
        }
    "#;
    let t = checked(src);
    let body = &t.funcs[0].body;
    let mut issues = Vec::new();
    let mut endings = Vec::new();
    for (point, stmt) in body.iter().enumerate() {
        match stmt {
            TStmt::ClassBorrow { alias, owner, mutable, value } => {
                assert_ne!(alias, owner, "non-owning alias and owner must have distinct slots");
                assert_eq!(value.ty, t.funcs[0].locals[*alias as usize]);
                issues.push((point, *alias, *owner, *mutable));
            },
            TStmt::ClassLoanEnd { alias, owner, mutable } => {
                endings.push((point, *alias, *owner, *mutable));
            },
            _ => {}
        }
    }
    assert_eq!(issues.len(), 2, "shared and unique borrow are both marked");
    assert_eq!(endings.len(), 2, "both last-use ends have typed evidence");
    assert!(!issues[0].3 && issues[1].3);
    for (end_point, alias, owner, mutable) in endings {
        let matching = issues.iter()
            .find(|(_, a, o, m)| *a == alias && *o == owner && *m == mutable)
            .expect("loan end must match original issuance");
        assert!(matching.0 < end_point, "cannot end loan before issuance");
    }
    let guest = atto_compiler::compile(src).expect("Wasm guest");
    assert!(guest.starts_with(b"\0asm"));
}

#[test]
fn expired_alias_is_never_implicitly_owned() {
    let src = r#"
        define class Parcel as
            pub let int value = 42;
        end
        pub fnc main(): int {
            val parcel = new Parcel();
            val reader = borrow! parcel;
            val bogus_owner = reader;
            return bogus_owner.value;
        }
    "#;
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_BORROW");
}

#[test]
fn explicit_capability_types_fail_closed_until_their_abi_is_verified() {
    for capability in ["BorrowMut<Person>",
                       "Borrow<Person, a>", "BorrowMut<Person, lifetime=a>",
                       "Shared<Person>", "Pointer<Person>"] {
        let source = format!(r#"
            define class Person as
                pub let int age = 42;
            end
            fnc inspect(item: {capability}): int {{ return 1; }}
            pub fnc main(): int {{ return 0; }}
        "#);
        assert_eq!(atto_compiler::compile(&source).unwrap_err().code,
            "E_UNSUPPORTED_TYPE", "unproved capability admitted: {capability}");
    }
}

#[test]
fn removed_pointer_and_borrow_sigils_are_syntax_errors() {
    for non_atto_type in ["&Person", "&mut Person", "*Person",
                          "*const Person", "*mut Person"] {
        let source = format!(r#"
            define class Person as
                pub let int age = 42;
            end
            fnc inspect(item: {non_atto_type}): int {{ return 1; }}
            pub fnc main(): int {{ return 0; }}
        "#);
        assert_eq!(atto_compiler::compile(&source).unwrap_err().code,
            "E_SYNTAX", "Rust pointer/borrow type accepted: {non_atto_type}");
    }
    for raw_expr in ["&person", "*person"] {
        let source = format!(r#"
            define class Person as
                pub let int age = 42;
            end
            pub fnc main(): int {{
                val person = new Person();
                val borrowed = {raw_expr};
                return borrowed.age;
            }}
        "#);
        assert_eq!(atto_compiler::compile(&source).unwrap_err().code,
            "E_SYNTAX", "Rust pointer/borrow expr accepted: {raw_expr}");
    }
}

#[test]
fn multiplication_remains_a_valid_arithmetic_operator() {
    let wasm = atto_compiler::compile("pub fnc main(): int { return 6 * 7; }")
        .expect("multiplication must not be disabled by pointer grammar");
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn readonly_borrow_formal_does_not_implicitly_move_caller_owner() {
    let source = r#"
        define class Person as pub let int age = 42; end
        fnc inspect(person: Borrow<Person>): int { return person.age; }
        pub fnc main(): int {
            let person = new Person();
            let a = inspect(person);
            let b = inspect(person);
            return a + b - person.age;
        }
    "#;
    let guest = atto_compiler::compile(source).expect("readonly Borrow<Person> must compile");
    assert!(guest.starts_with(b"\0asm"));
}

#[test]
fn readonly_borrow_formal_rejects_escapes_mutation_and_unproven_call_abi() {
    let tests = [
        (r#"define class Person as pub let int age = 42; end
            fnc steal(p: Borrow<Person>): Person { return p; }
            pub fnc main(): int { let p = new Person(); let q = steal(p); return q.age; }"#, "E_BORROW"),
        (r#"define class Person as pub let int age = 42; end
            fnc change(p: Borrow<Person>): void { p.age = 1; }
            pub fnc main(): int { let p = new Person(); change(p); return 1; }"#, "E_BORROW"),
        (r#"define class Person as pub let int age = 42; end
            fnc inspect(p: Borrow<Person>): int { return p.age; }
            pub fnc main(): int { return inspect(new Person()); }"#, "E_BORROW"),
        (r#"define class Person as pub let int age = 42; end
            fnc inspect(): Borrow<Person> { return new Person(); }
            pub fnc main(): int { return 0; }"#, "E_UNSUPPORTED_TYPE"),
        (r#"define class Person as pub let int age = 42; end
            fnc consume(p: Person): int { return p.age; }
            fnc inspect(p: Borrow<Person>): int { return consume(p); }
            pub fnc main(): int { let p = new Person(); return inspect(p); }"#, "E_BORROW"),
    ];
    for (source, code) in tests {
        assert_eq!(atto_compiler::compile(source).unwrap_err().code, code);
    }
}

#[test]
fn multi_argument_auto_borrow_supports_mixed_orders_and_shared_owners() {
    let cases = [
        r#"define class Person as pub let int age = 42; end
            fnc inspect(p: Borrow<Person>, n: int): int { return p.age + n; }
            pub fnc main(): int {
                let p = new Person();
                let a = inspect(p, 0);
                let b = inspect(p, 0);
                return a + b - p.age;
            }"#,
        r#"define class Person as pub let int age = 21; end
            fnc inspect(a: Borrow<Person>, n: int, b: Borrow<Person>): int {
                return a.age + b.age + n;
            }
            pub fnc main(): int {
                let p = new Person();
                let sum = inspect(p, 0, p);
                let owned = take! p;
                return sum + owned.age - 21;
            }"#,
        r#"define class Person as pub let int age = 20; end
            define class Pet as pub let int years = 22; end
            fnc sum(p: Borrow<Person>, pet: Borrow<Pet>): int {
                return p.age + pet.years;
            }
            pub fnc main(): int {
                let p = new Person();
                let pet = new Pet();
                return sum(p, pet) + p.age - p.age;
            }"#,
    ];
    for source in cases {
        let result = atto_compiler::compile(source).expect("multi-argument borrow must compile");
        assert!(result.starts_with(b"\0asm"));
    }
}

#[test]
fn call_loans_deny_moves_exclusive_access_and_unproven_reborrows() {
    let cases = [
        (r#"define class Person as pub let int age = 42; end
            fnc pair(a: Borrow<Person>, b: Person): int { return a.age + b.age; }
            pub fnc main(): int {
                let p = new Person();
                return pair(p, take! p);
            }"#, "E_BORROW"),
        (r#"define class Person as pub let int age = 42; end
            fnc pair(a: Person, b: Borrow<Person>): int { return a.age + b.age; }
            pub fnc main(): int {
                let p = new Person();
                return pair(take! p, p);
            }"#, "E_MOVED"),
        (r#"define class Person as pub int age: 42; end
            fnc inspect(p: Borrow<Person>, n: int): int { return p.age + n; }
            pub fnc main(): int {
                let p = new Person();
                val writer = borrow! mut p;
                let value = inspect(p, 0);
                writer.age = value;
                return p.age;
            }"#, "E_BORROW"),
        (r#"define class Person as pub let int age = 42; end
            fnc inspect(p: Borrow<Person>, n: int): int { return p.age + n; }
            pub fnc main(): int {
                let p = new Person();
                val reader = borrow! p;
                return inspect(reader, 0);
            }"#, "E_BORROW"),
        (r#"define class Person as pub let int age = 42; end
            fnc consume(p: Person): int { return p.age; }
            fnc inspect(p: Borrow<Person>, n: int): int { return p.age + n; }
            pub fnc main(): int {
                let p = new Person();
                return inspect(p, consume(p));
            }"#, "E_BORROW"),
    ];
    for (source, code) in cases {
        assert_eq!(atto_compiler::compile(source).unwrap_err().code, code);
    }
}

#[test]
fn inferred_call_loans_never_create_phantom_owned_object_locals() {
    let before = checked(r#"
        define class Person as pub let int age = 42; end
        fnc inspect(p: Borrow<Person>): int { return p.age; }
        pub fnc main(): int {
            let person = new Person();
            return person.age;
        }
    "#);
    let after = checked(r#"
        define class Person as pub let int age = 42; end
        fnc inspect(p: Borrow<Person>): int { return p.age; }
        pub fnc main(): int {
            let person = new Person();
            inspect(person);
            inspect(person);
            inspect(person);
            return person.age;
        }
    "#);
    let baseline = &before.funcs[before.main as usize].locals;
    let borrowed = &after.funcs[after.main as usize].locals;
    assert_eq!(borrowed, baseline,
        "non-owning call loans must not manufacture uninitialized owned class locals");
}


#[test]
fn explicit_and_implicit_class_call_loans_have_identical_guest_bytes() {
    // A successful shorthand must issue the same loan as the explicit
    // borrow! spelling, with no new owned class slot or runtime clone.
    // This proves only the narrow Borrow<Person> synchronous call profile,
    // NOT the future "fnc inspect(Person p)" default.
    let header = r#"
        define class Person as pub let int age = 42; end
        fnc inspect(p: Borrow<Person>): int { return p.age; }
    "#;
    let implicit = format!(r#"{header}
        pub fnc main(): int {{
            let person = new Person();
            let n = inspect(person);
            return n;
        }}
    "#);
    let explicit = format!(r#"{header}
        pub fnc main(): int {{
            let person = new Person();
            let n = inspect(borrow! person);
            return n;
        }}
    "#);
    let implicit_ir = checked(&implicit);
    let explicit_ir = checked(&explicit);
    assert_eq!(implicit_ir.funcs.len(), explicit_ir.funcs.len());
    for (a, b) in implicit_ir.funcs.iter().zip(explicit_ir.funcs.iter()) {
        assert_eq!(a.locals, b.locals, "borrow! must not invent owned locals");
    }
    let a = atto_compiler::compile(&implicit).expect("implicit class call loan");
    let b = atto_compiler::compile(&explicit).expect("explicit class call loan");
    assert_eq!(a, b, "call-site borrow! must not change the guest binary");
}

#[test]
fn returned_class_loans_and_unproved_lifetime_signatures_still_fail_closed() {
    // A scoped argument loan is NOT a returned-reference lifetime proof.
    // The target v1 Borrow<Child> result contract cannot be lowered until
    // owner-origin metadata survives the typed IR, cleanup and caller frame.
    let result_cases = [
        r#"
            define class Person as pub let int age = 42; end
            fnc view(p: Borrow<Person>): Borrow<Person> { return p; }
            pub fnc main(): int { return 0; }
        "#,
        r#"
            define class Person as pub let int age = 42; end
            fnc view(p: Person): Borrow<Person> { return p; }
            pub fnc main(): int { return 0; }
        "#,
        r#"
            define class Person as pub let int age = 42; end
            define class Reader as
                pub view(p: Borrow<Person>): Borrow<Person> { return p; }
            end
            pub fnc main(): int { return 0; }
        "#,
    ];
    for source in result_cases {
        let err = atto_compiler::compile(source).expect_err(
            "unsupported returned borrow must fail before guest output");
        assert_eq!(err.code, "E_UNSUPPORTED_TYPE",
            "a returned Borrow<T> must not be erased into an owned handle");
    }
}
