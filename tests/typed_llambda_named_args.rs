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
    assert!(wasm.starts_with(b"\\0asm"), "{source}");
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
    assert_eq!(params, &["a", "b"]);
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
fn type_annotation_errors_and_limits_are_explicit() {
    let first = "pub fnc main(): int { val c = |int a, a: int| : int -> a; return 0; }";
    assert_eq!(Parser::new(lexer::lex(first).unwrap()).parse_program().unwrap_err().code, "E_DUPLICATE");
    bad("const fnc c = |a| : int -> a; pub fnc main(): int { return 0; }", "E_INFER");
    bad("const fnc c = |void a| : int -> 1; pub fnc main(): int { return 0; }", "E_TYPE");
    bad("const fnc c = |List<int> a| : int -> 1; pub fnc main(): int { return 0; }", "E_UNSUPPORTED_TYPE");
    bad("pub fnc main(): int { stdio.println(value: 42); return 0; }", "E_NAMED_ARG");
}
