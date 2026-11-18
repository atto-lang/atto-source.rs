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


#[test]
fn mutable_class_loan_cannot_bypass_sealed_state_write() {
    // This previously reached the ordinary FieldSet path for a mutable alias
    // inside its own class, bypassing the critical-write lease.
    let source = r#"
        define state Counter as
            state { let int count: 0; }
            pub modify_another(Counter other): int {
                val alias = borrow! mut other;
                alias.count = 7;
                return 1;
            }
        end
        pub fnc main(): int { return 0; }
    "#;
    assert_eq!(compile(source).unwrap_err().code, "E_STATE_BORROW");
}

#[test]
fn borrowed_state_container_cannot_invoke_unproven_write_effects() {
    // A shared Borrow<Counter> is read-only even when an instance method
    // returns just void or int: it can transitively enter critical-write.
    let source = r#"
        define state Counter as
            state { let int count: 0; }
            pub increment(int amount): void {
                handle_state(critical-write |s| -> { s.count = s.count + amount; });
            }
        end
        fnc inspect(p: Borrow<Counter>): int {
            p.increment(1);
            return 1;
        }
        pub fnc main(): int {
            let counter = new Counter();
            return inspect(counter);
        }
    "#;
    assert_eq!(compile(source).unwrap_err().code, "E_STATE_BORROW");
}

#[test]
fn outstanding_shared_call_loan_blocks_state_method_on_original_owner() {
    // The first argument holds a read loan through all later argument
    // evaluation. A state method in the second argument may write that owner.
    let source = r#"
        define state Counter as
            state { let int count: 0; }
            pub bump(): int {
                handle_state(critical-write |s| -> { s.count = s.count + 1; });
                return 1;
            }
        end
        fnc inspect(p: Borrow<Counter>, n: int): int { return n; }
        pub fnc main(): int {
            let counter = new Counter();
            return inspect(counter, counter.bump());
        }
    "#;
    assert_eq!(compile(source).unwrap_err().code, "E_STATE_BORROW");
}

#[test]
fn ordinary_owned_state_critical_writes_still_compile() {
    // Do not disable state writes; require the compiler-issued lease on the
    // exact owned receiver and preserve normal method dispatch.
    let source = r#"
        define state Counter as
            state { let int count: 0; }
            pub bump(): int {
                handle_state(critical-write |s| -> { s.count = s.count + 1; });
                return handle_state(critical-read |s| -> s.count);
            }
        end
        pub fnc main(): int {
            let counter = new Counter();
            return counter.bump();
        }
    "#;
    let wasm = compile(source).expect("owned state + critical-write remains legal");
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn non_state_readonly_class_method_remains_callable_via_borrow() {
    // Normal class methods have immutable self in the current guest profile;
    // the new conservative state-container gate must not block them.
    let source = r#"
        define class Person as
            pub let int age = 42;
            pub get_age(): int { return self.age; }
        end
        fnc inspect(p: Borrow<Person>): int {
            return p.get_age();
        }
        pub fnc main(): int {
            let p = new Person();
            return inspect(p);
        }
    "#;
    let wasm = compile(source).expect("readonly class receiver should compile");
    assert!(wasm.starts_with(b"\0asm"));
}
