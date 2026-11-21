//! Bounded monomorphic Any specialization is checked by the existing
//! parser, typechecker and real guest Wasm emitter; no dynamic Any ABI is
//! invented by this first executable preview.
use atto_compiler::compile;

const SITE: &str = include_str!("playground_previews/match-types-is.atto");
fn pass(src: &str) {
    let wasm = compile(src).unwrap_or_else(|e| panic!("expected a real compiled guest: {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile(src).unwrap(), "compiler must be deterministic");
}
fn err(src: &str, want: &str) {
    let d = compile(src).expect_err("unsupported or unsafe Any should emit no guest");
    assert_eq!(d.code, want, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0);
}

#[test]
fn exact_published_type_match_is_an_actual_compiled_wasm_guest() {
    pass(SITE);
}

#[test]
fn statically_known_string_and_bool_tags_select_matching_type_arms() {
    pass(
        r#"
        fnc read(Any value): int {
            match value over
              on is int -> { return 10; }
              on is string -> { return 42; }
              on _ -> { return -1; }
            end
        }
        pub fnc main(): int {
            val message: Any = "hello";
            if message is int then return 0; fi
            return read(message);
        }
    "#,
    );
    pass(
        r#"
        fnc read(Any value): int {
            match value over
              on is bool when value -> { return 42; }
              on is bool -> { return 10; }
              on _ -> { return -1; }
            end
        }
        pub fnc main(): int {
            val flag: Any = true;
            return read(flag);
        }
    "#,
    );
}

#[test]
fn non_literal_or_escaping_any_cannot_masquerade_as_scalar_wasm() {
    err(
        r#"
        fnc read(Any v): int { return 42; }
        pub fnc main(): int {
            val n: Any = 1 + 2;
            return read(n);
        }
    "#,
        "E_UNSUPPORTED_TYPE",
    );
    err(
        r#"
        fnc read(Any v): int { return 42; }
        pub fnc main(): int {
            val n: Any = new Object();
            return read(n);
        }
    "#,
        "E_UNSUPPORTED_TYPE",
    );
    err(
        r#"
        pub fnc main(): int {
            val n: Any = 42;
            return n;
        }
    "#,
        "E_UNSUPPORTED",
    );
    err(
        r#"
        fnc read(Any v): int { return 42; }
        pub fnc main(): int { return read(42); }
    "#,
        "E_UNSUPPORTED",
    );
}

#[test]
fn mismatched_concrete_calls_and_unhandled_type_tests_fail() {
    // Source-local Any does not imply a dynamic runtime tag or a host ABI.
    err(
        r#"
        fnc read(Any v): int {
            match v over on is string -> { return 1; } on _ -> { return 0; } end
        }
        pub fnc main(): int {
            val value: Any = 42;
            return read("wrong");
        }
    "#,
        "E_UNSUPPORTED",
    );
    err(
        r#"
        fnc read(Any v): int {
            match v over on is float -> { return 42; } end
        }
        pub fnc main(): int {
            val value: Any = 42;
            return read(value);
        }
    "#,
        "E_MATCH",
    );
    err(
        r#"
        fnc read(Any v): int {
            match v over on is int when v > 0 -> { return 42; } end
        }
        pub fnc main(): int {
            val value: Any = 42;
            return read(value);
        }
    "#,
        "E_MATCH",
    );
}

#[test]
fn legacy_match_and_other_published_guests_remain_compilable() {
    pass(include_str!("playground_previews/match-values.atto"));
    pass(include_str!("playground_previews/fib-match.atto"));
    pass("pub fnc main(): int { return 42; }");
}
