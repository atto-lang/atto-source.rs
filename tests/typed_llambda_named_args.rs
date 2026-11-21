//! Conformance for llambda parameter spellings and statically bound named calls.
//! Executable scope: source compiler's scalar Rust/Wasm ABI, not general closures.
use atto_compiler::{ast::{ExprKind, Stmt, Type}, lexer, parser::Parser};

fn ast(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).unwrap()).parse_program()
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn ok(source: &str) {
    let wasm = atto_compiler::compile(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"));
    assert!(wasm.starts_with(b"\0asm"), "{source}");
    assert_eq!(wasm, atto_compiler::compile(source).unwrap(),
        "compilation must be deterministic");
}

fn bad(source: &str, code: &str) {
    let err = atto_compiler::compile(source)
        .expect_err("invalid programs must not emit executable artifacts");
    assert_eq!(err.code, code, "{source}: {err}");
}

#[test]
fn both_user_requested_constant_function_styles_compile() {
    for signature in ["int a, int b", "a: int, b: int"] {
        let source = format!(
            "const fnc c = |{signature}| : int -> a + b; pub fnc main(): int {{ return c(b: 22, a: 20); }}"
        );
        ok(&source);
        let program = ast(&source);
        assert_eq!(program.funcs[0].params.len(), 2);
        assert_eq!(program.funcs[0].params[0].name, "a");
        assert_eq!(program.funcs[0].params[0].ty, Type::Int);
        assert_eq!(program.funcs[0].params[1].name, "b");
        assert_eq!(program.funcs[0].ret, Type::Int);
    }
}

#[test]
fn all_parameter_spelling_mixes_canonicalize() {
    let source = "pub fnc main(): int { val c = |int a, b: int| : int -> a + b; return c(a: 19, b: 23); }";
    let p = ast(source);
    let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else { panic!("missing lambda") };
    let ExprKind::Lambda { params, param_types, .. } = &init.kind else { panic!("not a lambda") };
    assert_eq!(params, &vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(param_types, &[Some("int".to_string()), Some("int".to_string())]);
    ok(source);
}

#[test]
fn local_const_fncs_are_closed_and_statically_callable() {
    ok("pub fnc main(): int { const fnc c = |int a, b: int| : int -> a + b; return c(b: 21, a: 21); }");
    bad("pub fnc main(): int { val extra = 1; const fnc c = |int a| : int -> a + extra; return c(a: 41); }", "E_CONST");
}

#[test]
fn ordinary_functions_take_named_and_mixed_arguments() {
    ok("fnc sum(int a, b: int): int { return a + b; } pub fnc main(): int { return sum(b: 22, a: 20); }");
    ok("fnc sum(int a, int b): int { return a + b; } pub fnc main(): int { return sum(20, b: 22); }");
    ok("fnc sum(a: int, b: int): int { return a + b; } pub fnc main(): int { val x = 21; return sum(a: x, b: x); }");
}

#[test]
fn explicit_lambda_types_check_and_old_inference_remains_valid() {
    ok("pub fnc main(): int { val c = |a: int, int b| : int -> a + b; return c(21, 21); }");
    ok("pub fnc main(): int { val c = |a, b| : int -> a + b; return c(21, 21); }");
    bad("pub fnc main(): int { val c = |a: bool| : int -> a + 1; return c(true); }", "E_TYPE");
    bad("const fnc c = |int a| : bool -> a + 1; pub fnc main(): int { return 0; }", "E_TYPE");
}

#[test]
fn malformed_calls_fail_closed_before_codegen() {
    let pre = "const fnc c = |int a, int b| : int -> a + b; pub fnc main(): int { return ";
    for (call, code) in [
        ("c(a: 1, a: 2)", "E_DUPLICATE"),
        ("c(z: 1, a: 2)", "E_NAMED_ARG"),
        ("c(a: 1)", "E_ARITY"),
        ("c(a: 1, 2)", "E_SYNTAX"),
        ("c(1, a: 2)", "E_DUPLICATE"),
        ("c(b: true, a: 2)", "E_TYPE"),
    ] {
        bad(&format!("{pre}{call}; }}"), code);
    }
}

#[test]
fn no_reordering_of_effectful_named_arguments_without_temporary_slots() {
    let prefix = "fnc f(int a, int b): int { return a + b; } fnc tick(): int { return 1; } pub fnc main(): int { return ";
    bad(&format!("{prefix}f(b: tick(), a: 2); }}"), "E_EFFECT_ORDER");
    // An already correctly ordered call is safe and evaluates each argument once.
    ok(&format!("{prefix}f(a: tick(), b: 41); }}"));
}

#[test]
fn class_methods_and_constructor_names_bind_without_abi_changes() {
    ok(r#"
        define class Calculator as
            pub fnc add(int a, int b): int { return a + b; }
        end
        pub fnc main(): int {
            val calc = new Calculator();
            return calc.add(b: 22, a: 20);
        }
    "#);
    ok(r#"
        define class Empty as
            constructor(int x, int y) { }
        end
        pub fnc main(): int {
            val item = new Empty(y: 22, x: 20);
            return 42;
        }
    "#);
}

#[test]
fn type_annotation_errors_and_limits_are_explicit() {
    let first = "pub fnc main(): int { val c = |int a, a: int| : int -> a; return 0; }";
    assert_eq!(Parser::new(lexer::lex(first).unwrap()).parse_program().unwrap_err().code, "E_DUPLICATE");
    bad("const fnc c = |a| : int -> a; pub fnc main(): int { return 0; }", "E_INFER");
    bad("const fnc c = |void a| : int -> 1; pub fnc main(): int { return 0; }", "E_TYPE");
    bad("const fnc c = |List<int> a| : int -> 1; pub fnc main(): int { return 0; }", "E_UNSUPPORTED_TYPE");
    bad("pub fnc main(): int { stdio.println(value: 42); return 0; }", "E_NAMED_ARG");
}

#[test]
fn four_requested_void_lambda_headers_parse_to_identical_formals() {
    // This is parser coverage for local const syntax, not evidence
    // that every first-class callable/capture profile compiles or runs.
    let variants = [
        ("|int foo, string x| -> { return; };", None),
        ("|foo: int, x: string| -> { return; }", None),
        ("|foo: int, x: string| -> void { return; }", Some("void")),
        ("|foo: int, x: string| : void -> { return; }", Some("void")),
    ];
    for (header, result) in variants {
        let source = format!(
            "pub fnc main(): int {{\nconst v = {header}\nreturn 42;\n}}\n"
        );
        let program = ast(&source);
        let Stmt::Bind { init, .. } = &program.funcs[0].body[0] else {
            panic!("lambda binding missing: {header}");
        };
        let ExprKind::Lambda { params, param_types, return_type, body } = &init.kind else {
            panic!("expected a lambda for {header}");
        };
        assert_eq!(params, &["foo".to_owned(), "x".to_owned()], "{header}");
        assert_eq!(param_types, &[Some("int".into()), Some("string".into())], "{header}");
        assert_eq!(return_type.as_deref(), result, "{header}");
        let atto_compiler::ast::LambdaBody::Block(statements) = body else {
            panic!("return body was not preserved: {header}");
        };
        assert!(matches!(statements.as_slice(), [Stmt::Return(None, _)]), "{header}");
    }
}

#[test]
fn all_four_void_lambda_styles_lower_through_the_direct_call_profile() {
    // This verifies compile-time Wasm generation and determinism. A separate
    // executed guest oracle must prove actual return behavior at exact HEAD.
    for body in [
        "|int foo, string x| -> { return; };",
        "|foo: int, x: string| -> { return; }",
        "|foo: int, x: string| -> void { return; }",
        "|foo: int, x: string| : void -> { return; }",
    ] {
        let source = format!(
            "pub fnc main(): int {{\n const v = {body}\n v(5, \"ok\");\n return 42;\n}}"
        );
        ok(&source);
    }
}

#[test]
fn explicit_void_lambda_rejects_value_return_and_void_formal() {
    bad(
        "pub fnc main(): int { const v = |foo: int| : void -> { return 7; }; v(5); return 42; }",
        "E_TYPE",
    );
    bad(
        "pub fnc main(): int { const v = |void foo| : void -> { return; }; v(5); return 42; }",
        "E_TYPE",
    );
}

#[test]
fn closed_const_lambda_infers_ignored_formal_at_direct_call() {
    // The user-requested |_| spelling remains a ONE-parameter callable,
    // and : int declares the RESULT rather than inventing a parameter type.
    // A literal direct call is sufficient to infer its safe scalar ABI.
    ok("pub fnc main(): int { const fnc v = |_| : int -> 5; return v(41); }");
    ok("pub fnc main(): int { const fnc v = |_| : int -> 5; return v(true); }");
    ok("pub fnc main(): int { const fnc v = || : int -> 5; return v(); }");
    ok("pub fnc main(): int { const fnc v = |x| : int -> x + 1; return v(41); }");
    // A module-scope constant cannot infer parameter ABI from a future call.
    ok("const fnc v = || : int -> 5; pub fnc main(): int { return v(); }");
    ok("const fnc v = |int _| : int -> 5; pub fnc main(): int { return v(41); }");
    bad("pub fnc main(): int { const fnc v = |_| : int -> 5; return 0; }", "E_UNSUPPORTED");
    bad("pub fnc main(): int { const fnc v = |_| : int -> 5; return v(missing); }", "E_INFER");
    bad("pub fnc main(): int { const fnc v = |_| : int -> 5; return v(); }", "E_ARITY");
    bad("pub fnc main(): int { val runtime = 41; const fnc v = |_| : int -> runtime; return v(42); }", "E_CONST");
}

#[test]
fn repeated_ignored_formals_have_distinct_private_abi_slots() {
    // Each '_' is a separate argument position, not a duplicate name. The
    // lowered callable must not expose those private slots as source names.
    let source = "pub fnc main(): int { const fnc v = |_, _| : int -> 5; return v(41, false); }";
    let program = ast(source);
    let Stmt::Bind { init, .. } = &program.funcs[0].body[0] else {
        panic!("missing const lambda binding")
    };
    let ExprKind::Lambda { params, .. } = &init.kind else {
        panic!("expected pipe lambda")
    };
    assert_eq!(params.len(), 2);
    assert_ne!(params[0], params[1]);
    assert!(params.iter().all(|name| name.starts_with("$ignored_lambda_")));
    ok(source);

    ok("const fnc v = |int _, bool _| : int -> 5; pub fnc main(): int { return v(41, false); }");
    bad("pub fnc main(): int { const fnc v = |_, _| : int -> 5; return v(41); }", "E_ARITY");
    bad("pub fnc main(): int { const fnc v = |_, _| : int -> _; return v(41, false); }", "E_CONST");
}

#[test]
fn bare_lambda_heads_never_emit_a_wasm_artifact() {
    for expression in ["_ : int -> 5", "_ -> 5", "x : int -> x + 5", "x -> x + 5"] {
        let source = format!(
            "pub fnc main(): int {{ const fnc v = {expression}; return 0; }}"
        );
        assert!(
            atto_compiler::compile(&source).is_err(),
            "delimiter-free lambda compiled: {expression}"
        );
    }
}
