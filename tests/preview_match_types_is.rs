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
    pass(r#"
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
    "#);
    pass(r#"
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
    "#);
}

#[test]
fn signed_numeric_any_literals_compile_as_checked_monomorphic_guests() {
    // The published playground example previously rejected the two-token -42.
    let negative_site = SITE.replace("val value: Any = 42;", "val value: Any = -42;");
    assert_ne!(negative_site, SITE, "the published fixture changed unexpectedly");
    pass(&negative_site);

    for number in ["-42", "-0", "-9_223_372_036_854_775_808", "-0b101010"] {
        let source = format!(r#"
            fnc classify(Any value): int {{
                match value over
                    on is int when value > 0 -> {{ return 1; }}
                    on is int -> {{ return 42; }}
                    on _ -> {{ return 0; }}
                end
            }}
            pub fnc main(): int {{
                val value: Any = {number};
                if value is int && value eq {number} then
                    return classify(value);
                fi
                return 0;
            }}
        "#);
        pass(&source);
    }

    pass(r#"
        fnc classify(Any value): int {
            match value over
                on is float -> { return 42; }
                on _ -> { return 0; }
            end
        }
        pub fnc main(): int {
            val value: Any = -3.5;
            return classify(value);
        }
    "#);
}

#[test]
fn signed_any_literal_proof_still_rejects_computation_and_unsafe_shapes() {
    for expr in ["-true", "-\"text\"", "-42 + 1", "-42 * 2", "-(42)", "--42"] {
        let source = format!(r#"
            fnc classify(Any value): int {{ return 42; }}
            pub fnc main(): int {{
                val value: Any = {expr};
                return classify(value);
            }}
        "#);
        err(&source, "E_UNSUPPORTED_TYPE");
    }
    err(r#"
        fnc classify(Any value): int { return 42; }
        pub fnc main(): int {
            val value: Any = -9223372036854775809;
            return classify(value);
        }
    "#, "E_INTEGER_RANGE");
}

#[test]
fn non_literal_or_escaping_any_cannot_masquerade_as_scalar_wasm() {
    err(r#"
        fnc read(Any v): int { return 42; }
        pub fnc main(): int {
            val n: Any = 1 + 2;
            return read(n);
        }
    "#, "E_UNSUPPORTED_TYPE");
    err(r#"
        fnc read(Any v): int { return 42; }
        pub fnc main(): int {
            val n: Any = new Object();
            return read(n);
        }
    "#, "E_UNSUPPORTED_TYPE");
    err(r#"
        pub fnc main(): int {
            val n: Any = 42;
            return n;
        }
    "#, "E_UNSUPPORTED");
    err(r#"
        fnc read(Any v): int { return 42; }
        pub fnc main(): int { return read(42); }
    "#, "E_UNSUPPORTED");
}

#[test]
fn mismatched_concrete_calls_and_unhandled_type_tests_fail() {
    // Source-local Any does not imply a dynamic runtime tag or a host ABI.
    err(r#"
        fnc read(Any v): int {
            match v over on is string -> { return 1; } on _ -> { return 0; } end
        }
        pub fnc main(): int {
            val value: Any = 42;
            return read("wrong");
        }
    "#, "E_UNSUPPORTED");
    err(r#"
        fnc read(Any v): int {
            match v over on is float -> { return 42; } end
        }
        pub fnc main(): int {
            val value: Any = 42;
            return read(value);
        }
    "#, "E_MATCH");
    err(r#"
        fnc read(Any v): int {
            match v over on is int when v > 0 -> { return 42; } end
        }
        pub fnc main(): int {
            val value: Any = 42;
            return read(value);
        }
    "#, "E_MATCH");
}

#[test]
fn legacy_match_and_other_published_guests_remain_compilable() {
    pass(include_str!("playground_previews/match-values.atto"));
    pass(include_str!("playground_previews/fib-match.atto"));
    pass("pub fnc main(): int { return 42; }");
}
