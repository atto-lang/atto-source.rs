//! Routine direct calls and bounded compile-time named callable specialization.
use atto_compiler::compile;
const SITE: &str = include_str!("playground_previews/routine.atto");
fn pass(src: &str) {
    let guest = compile(src).unwrap_or_else(|e| panic!("expected valid Wasm: {e}"));
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(guest, compile(src).unwrap());
}
fn fail(src: &str, code: &str) {
    let e = compile(src).expect_err("invalid callable must not emit guest");
    assert_eq!(e.code, code, "{e}");
}
#[test]
fn exact_website_routine_and_named_higher_order_guest_compile() {
    pass(SITE);
}
#[test]
fn routine_cannot_escape_into_first_class_parameter() {
    fail(&SITE.replace("apply(twice, 21)", "apply(square, 21)"), "E_ROUTINE");
}
#[test]
fn end_bodied_routines_do_not_scan_into_the_following_main() {
    pass(
        r#"
        routine r_end() -> int ::
            let int n = 3,
            n + 2
        end
        pub fnc main(): int { return r_end() + 37; }
    "#,
    );
    fail(
        r#"
        routine r_recur() -> int :: r_recur() end
        pub fnc main(): int { return 0; }
    "#,
        "E_ROUTINE_RECURSION",
    );
}

#[test]
fn recursive_routines_are_rejected_before_guest_emission() {
    fail(
        r#"
        routine recurse(int n): int { return recurse(n - 1); }
        pub fnc main(): int { return recurse(2); }
    "#,
        "E_ROUTINE_RECURSION",
    );
}
#[test]
fn only_statically_known_matching_callback_functions_are_lowered() {
    fail(&SITE.replace("apply(twice, 21)", "apply(42, 21)"), "E_UNSUPPORTED");
    fail(&SITE.replace("apply(twice, 21)", "apply(unknown, 21)"), "E_TYPE");
    fail(&SITE.replace("Fnc<int, int>", "Fnc<int, bool>"), "E_UNSUPPORTED_TYPE");
}
#[test]
fn generated_callback_names_cannot_be_forged() {
    fail(
        r#"
        pub fnc main(): int {
            val __atto_internal_direct_cb_5_apply_5_twice = 0;
            return 42;
        }
        routine foo(): int { return 0; }
    "#,
        "E_RESERVED",
    );
    pass("pub fnc main(): int { return 42; }");
}

#[test]
fn mutually_recursive_routines_fail_closed_even_without_direct_self_calls() {
    fail(
        r#"
        routine even(int n): int {
            if n == 0 then return 1; fi
            return odd(n - 1);
        }
        routine odd(int n): int {
            if n == 0 then return 0; fi
            return even(n - 1);
        }
        pub fnc main(): int { return even(4); }
    "#,
        "E_ROUTINE_RECURSION",
    );
}

#[test]
fn indirect_routine_recursion_through_ordinary_function_is_forbidden() {
    fail(
        r#"
        routine start(int n): int { return bridge(n); }
        fnc bridge(int n): int { return start(n); }
        pub fnc main(): int { return start(2); }
    "#,
        "E_ROUTINE_RECURSION",
    );
}

#[test]
fn acyclic_routine_call_chain_still_compiles_without_false_recursion() {
    pass(
        r#"
        routine increment(int n): int { return n + 1; }
        fnc bridge(int n): int { return increment(n); }
        routine double_bridge(int n): int { return bridge(n) * 2; }
        pub fnc main(): int { return double_bridge(20); }
    "#,
    );
}
