//! Focused lifetime-safety regressions for the *current* Borrow<Class> call ABI.
//! These tests do NOT claim that bare class formals borrow by default or that
//! returning borrowed values / named lifetime parameters is implemented.
use atto_compiler::compile;

#[test]
fn mutable_readonly_borrow_formals_fail_before_guest_emission() {
    // Both accepted parameter-order spellings must be rejected rather than
    // silently downgraded to an immutable Borrow<T> in body_check().
    for formal in ["p: Borrow<Person> mut", "Borrow<Person> mut p"] {
        let source = format!(r#"
            define class Person as pub let int age = 42; end
            fnc inspect({formal}): int {{ return p.age; }}
            pub fnc main(): int {{
                let person = new Person();
                return inspect(person);
            }}
        "#);
        let err = compile(&source).expect_err(
            "mutable borrowed formal must not be admitted as a read-only ABI");
        assert_eq!(err.code, "E_UNSUPPORTED_TYPE", "{formal}: {err:?}");
    }
}

#[test]
fn nested_shared_call_loans_keep_one_owner_and_retire_after_call() {
    // The outer read loan remains active through evaluation of the nested
    // argument, but a second shared read is compatible. Both retire before
    // take! transfers ownership; no additional owned class local is needed.
    let header = r#"
        define class Person as pub let int age = 42; end
        fnc inspect(p: Borrow<Person>, n: int): int { return p.age + n; }
    "#;
    let implicit = format!(r#"{header}
        pub fnc main(): int {{
            let person = new Person();
            let sum = inspect(person, inspect(person, 0));
            let owned = take! person;
            return sum - owned.age;
        }}
    "#);
    let explicit = format!(r#"{header}
        pub fnc main(): int {{
            let person = new Person();
            let sum = inspect(borrow! person, inspect(borrow! person, 0));
            let owned = take! person;
            return sum - owned.age;
        }}
    "#);
    let a = compile(&implicit).expect("nested inferred shared call loans");
    let b = compile(&explicit).expect("nested explicit shared call loans");
    assert!(a.starts_with(b"\0asm"));
    assert_eq!(a, b, "explicit borrowing must preserve the same guest ABI");
}

#[test]
fn nested_owned_argument_cannot_consume_outer_borrowed_owner() {
    let source = r#"
        define class Person as pub let int age = 42; end
        fnc inspect(p: Borrow<Person>, n: int): int { return p.age + n; }
        fnc consume(p: Person): int { return p.age; }
        pub fnc main(): int {
            let person = new Person();
            return inspect(person, consume(person));
        }
    "#;
    assert_eq!(compile(source).unwrap_err().code, "E_BORROW");
}

#[test]
fn old_bare_owned_formals_must_not_be_relabelled_as_verified_borrows() {
    // Migration gate: fnc consume(Person) still owns/moves on this compiler
    // branch. A versioned migration and an explicit consuming-formal syntax
    // must land before the default can safely change.
    let source = r#"
        define class Person as pub let int age = 42; end
        fnc consume(p: Person): int { return p.age; }
        pub fnc main(): int {
            let person = new Person();
            let n = consume(person);
            return n + person.age;
        }
    "#;
    assert_eq!(compile(source).unwrap_err().code, "E_MOVED");
}

#[test]
fn candidate_returned_borrow_still_requires_proven_lifetime_abi() {
    // Single-source lifetime elision and ambiguous multiple-source return
    // relationships both remain target contracts, not erased handles.
    for signature in [
        "fnc view(p: Borrow<Person>): Borrow<Person> { return p; }",
        "fnc choose(left: Borrow<Person>, right: Borrow<Person>): Borrow<Person> { return left; }",
    ] {
        let source = format!(r#"
            define class Person as pub let int age = 42; end
            {signature}
            pub fnc main(): int {{ return 0; }}
        "#);
        assert_eq!(compile(&source).unwrap_err().code, "E_UNSUPPORTED_TYPE");
    }
}
