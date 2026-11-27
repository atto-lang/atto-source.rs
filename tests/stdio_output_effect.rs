//! Browser-compiler subset evidence for stdout's non-suspending *admission*
//! spelling. This does not prove a native unbounded channel or OS flush.
use atto_compiler::compile;

#[test]
fn ordinary_stdout_writes_do_not_require_a_suspends_modifier() {
    let source = r#"pub fnc main(): int {
        writech stdio.out, "Hello";
        stdio.out << " world";
        (stdio.out) << 42;
        return 7;
    }"#;
    let explicit = r#"pub fnc main(): int {
        stdio.print("Hello");
        stdio.print(" world");
        stdio.print(42);
        return 7;
    }"#;
    let bytes = compile(source).expect("stdout admission must be usable by an ordinary fnc");
    let expected = compile(explicit).expect("checked host print is already supported");
    assert_eq!(bytes, expected,
        "all admitted stdout spellings must reuse the typed nonparking print import");
}

#[test]
fn pure_function_cannot_emit_even_to_non_suspending_stdio() {
    for statement in [
        r#"writech stdio.out, "Hello";"#,
        r#"stdio.out << "Hello";"#,
        r#"(stdio.out) << "Hello";"#,
        r#"stdio.print("Hello");"#,
    ] {
        let source = format!("pub pure fnc main(): int {{ {statement} return 7; }}");
        let err = compile(&source).expect_err("pure does not grant I/O authority");
        assert_eq!(err.code, "E_PURE", "{statement}: {err}");
    }
}

#[test]
fn stdout_non_suspending_exception_is_not_generalized_to_other_streams() {
    for statement in [
        r#"writech unknown, "Hello";"#,
        r#"output_stream << "Hello";"#,
        r#"stdio.err << "Hello";"#,
        r#"stdio.in << "Hello";"#,
    ] {
        let source = format!("pub fnc main(): int {{ {statement} return 7; }}");
        let err = compile(&source).expect_err("other transports are not browser-backed");
        assert!(matches!(err.code, "E_UNSUPPORTED" | "E_SYNTAX"),
            "{statement}: {err}");
    }
}

#[test]
fn canonical_stdout_methods_share_raw_string_print_lowering() {
    let source = r#"pub fnc main(): int { stdio.out.write("A"); process.stdout.write("B"); return 42; }"#;
    let legacy = r#"pub fnc main(): int { stdio.print("A"); stdio.print("B"); return 42; }"#;
    assert_eq!(compile(source).unwrap(), compile(legacy).unwrap());
}

#[test]
fn canonical_stdout_does_not_bypass_shadowing_purity_or_arity() {
    for (source, expected) in [
        (r#"pub fnc main(): int { val int process = 1; process.stdout.write("A"); return 42; }"#, "E_IO_SHADOW"),
        (r#"pub fnc main(): int { val int stdio = 1; stdio.out.write("A"); return 42; }"#, "E_IO_SHADOW"),
        (r#"pub pure fnc main(): int { process.stdout.write("A"); return 42; }"#, "E_PURE"),
        (r#"pub fnc main(): int { stdio.out.write("A","B"); return 42; }"#, "E_ARITY"),
        (r#"pub fnc main(): int { process.stdout.write(42); return 42; }"#, "E_UNSUPPORTED"),
    ] {
        let err = compile(source).expect_err("must fail closed");
        assert_eq!(err.code, expected, "{source}: {err}");
    }
}
