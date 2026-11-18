//! Comma-delimited `seq` in expression and function-argument positions.
//! Real Rust compiler -> checked AST -> deterministic guest Wasm admission.
use atto_compiler::{
    ast::{ExprKind, LambdaBody, Stmt},
    compile,
    lexer::lex,
    parser::Parser,
};

fn admitted(src: &str) {
    let first = compile(src).unwrap_or_else(|e| panic!("seq rejected: {e}\n{src}"));
    assert!(first.len() >= 8 && &first[..8] == b"\0asm\x01\0\0\0", "{src}");
    assert_eq!(first, compile(src).unwrap(), "nondeterministic guest bytes: {src}");
}

fn rejected(src: &str) {
    assert!(compile(src).is_err(), "bad seq compiled: {src}");
}

#[test]
fn checked_comma_sequences_in_calls_and_expression_values() {
    for src in [
        // A bare seq inside a call consumes the call's remaining argument commas.
        "fnc calculate(int v): int { return v; } pub fnc main(): int { return calculate(seq 1, 40 + 2); }",
        "fnc calculate(int v): int { return v; } pub fnc main(): int { return calculate(seq\n 1,\n 2,\n 42\n); }",
        // Delimited seq is one argument, permitting additional arguments.
        "fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add(seq(1, 2), 40); }",
        "fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add((> 1, 2), 40); }",
        "fnc add(int a, int b): int { return a + b; } pub fnc main(): int { return add(40, seq 1, 2); }",
        // seq expression usable as a value outside calls.
        "pub fnc main(): int { const int result = seq(1, 42); return result; }",
        "pub fnc main(): int { return seq(1, 42); }",
        "pub fnc main(): int { ^ seq(1, 42); }",
        "pub fnc main(): int { return (> 1, 42); }",
    ] { admitted(src); }
}

#[test]
fn local_assignment_steps_are_checked_and_yield_new_value() {
    for src in [
        "fnc foo(int v): int { return v * 2; } pub fnc main(): int { let int z = 10; const int x = 5; return foo(seq(z += x, z)); }",
        "fnc foo(int v): int { return v * 2; } pub fnc main(): int { let int z = 10; const int x = 5; return foo(seq z += x, z); }",
        "pub fnc main(): int { let int z = 10; return seq(z = 42, z); }",
        // Mutating a function-local variable does not by itself violate purity.
        "pure fnc p(): int { let int z = 41; return seq(z += 1, z); } pub fnc main(): int { return p(); }",
        "pub fnc main(): int { let int z = 10; return (> z += 32, z); }",
    ] { admitted(src); }
}

#[test]
fn parsed_seq_is_not_a_tuple_or_a_multi_argument_call() {
    let src = "fnc foo(int v): int { return v; } pub fnc main(): int { return foo(seq 1, 2, 42); }";
    let ast = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    let main = ast.funcs.iter().find(|f| f.name == "main").unwrap();
    let Stmt::Return(Some(expr), _) = &main.body[0] else { panic!("expected return"); };
    let ExprKind::Call(_, args) = &expr.kind else { panic!("expected call"); };
    assert_eq!(args.len(), 1, "bare seq commas must not become call arguments");
    let ExprKind::Sequence(steps, tail) = &args[0].kind else { panic!("expected checked seq AST"); };
    assert_eq!(steps.len(), 2);
    assert!(matches!(tail.kind, ExprKind::Int(Some(42))));
}

#[test]
fn reject_semicolons_bad_commas_and_assignment_escape() {
    for src in [
        "pub fnc main(): int { return seq(); }",
        "pub fnc main(): int { return seq(1,); }",
        "pub fnc main(): int { return seq(,1); }",
        "pub fnc main(): int { return seq(1,,2); }",
        "pub fnc main(): int { return seq(1; 42); }",
        "fnc f(int x): int { return x; } pub fnc main(): int { return f(seq 1; 42); }",
        "pub fnc main(): int { return seq(1, let x = 42); }",
        "pub fnc main(): int { const int z = 10; return seq(z += 1, z); }",
        "pub fnc main(): int { let int z = 10; return seq(z += 1); }",
        "pub fnc main(): int { let int z = 10; return seq(z += 1, z,); }",
        "pub fnc main(): int { let int z = 10; return seq(1 + z = 42, z); }",
        "pure fnc f(): int { return seq(stdio.println(1), 42); } pub fnc main(): int { return f(); }",
        "pub fnc main(): int { return seq(let x = 42, x) + x; }",
        "pub fnc main(): int { return (1, 42); }",
    ] { rejected(src); }
}

#[test]
fn braceless_lambda_comma_body_is_last_value_not_tuple() {
    // Lambda codegen is not yet available in this browser compiler.
    // Verify the real lexer/parser AST without falsely claiming guest execution.
    for src in [
        "pub fnc main(): int { val f = || -> 1, 2, 42; return 0; }",
        "pub fnc main(): int { val f = || -> seq(1, 2, 42); return 0; }",
        "pub fnc main(): int { val f = || -> (> 1, 2, 42); return 0; }",
    ] {
        let ast = Parser::new(lex(src).unwrap()).parse_program().unwrap();
        let Stmt::Bind { init, .. } = &ast.funcs[0].body[0] else { panic!("expected binding") };
        let ExprKind::Lambda { body: LambdaBody::Expression(value), .. } = &init.kind
            else { panic!("expected expression lambda: {src}") };
        let ExprKind::Sequence(prefix, tail) = &value.kind
            else { panic!("lambda must have explicit sequence AST: {src}") };
        assert_eq!(prefix.len(), 2, "{src}");
        assert!(matches!(tail.kind, ExprKind::Int(Some(42))), "{src}");
    }
}

#[test]
fn explicit_lambda_returns_use_same_checked_comma_sequence_ast() {
    // The browser compiler deliberately does not yet lower arbitrary lambdas.
    // Test parser AST only; NEVER count these as executed guest tests.
    for src in [
        "pub fnc main(): int { val f = (|| -> return 10, 20, 42;); return 0; }",
        "pub fnc main(): int { val f = (|| -> ^ 10, 20, 42;); return 0; }",
        "pub fnc main(): int { val f = (|| -> return seq 10, 20, 42;); return 0; }",
        "pub fnc main(): int { val f = (|| -> ^ seq 10, 20, 42;); return 0; }",
        "pub fnc main(): int { val f = (|| -> return seq(10, 20, 42);); return 0; }",
        "pub fnc main(): int { val f = (|| -> ^ seq(10, 20, 42);); return 0; }",
        "pub fnc main(): int { val f = (|| -> return ^ (10, 20, 42);); return 0; }",
    ] {
        let ast = Parser::new(lex(src).unwrap()).parse_program().unwrap();
        let Stmt::Bind { init, .. } = &ast.funcs[0].body[0] else { panic!("expected lambda binding: {src}") };
        let ExprKind::Lambda { body: LambdaBody::Statement(statement), .. } = &init.kind
            else { panic!("expected explicit-return lambda: {src}") };
        let Stmt::Return(Some(value), _) = statement.as_ref()
            else { panic!("expected explicit return: {src}") };
        let ExprKind::Sequence(prefix, tail) = &value.kind
            else { panic!("expected sequence, not tuple: {src}") };
        assert_eq!(prefix.len(), 2, "{src}");
        assert!(matches!(tail.kind, ExprKind::Int(Some(42))), "{src}");
    }
}

#[test]
fn malformed_explicit_lambda_comma_returns_fail_closed() {
    for src in [
        "pub fnc main(): int { val f = || -> return 1,; return 0; }",
        "pub fnc main(): int { val f = || -> ^ 1,,2; return 0; }",
        "pub fnc main(): int { val f = || -> return seq 1; 2; return 0; }",
        "pub fnc main(): int { val f = || -> return ^ (); return 0; }",
        "pub fnc main(): int { val f = || -> return 1, 2); return 0; }",
    ] {
        let lexed = lex(src).unwrap();
        assert!(Parser::new(lexed).parse_program().is_err(),
            "malformed explicit lambda return parsed: {src}");
    }
    let src = "pub fnc main(): int { val f = || -> return ^ (); return 0; }";
    let err = Parser::new(lex(src).unwrap()).parse_program().unwrap_err();
    assert_eq!(err.code, "E_EMPTY_SEQUENCE", "{err}");
}

#[test]
fn delimiters_scope_and_effects_hold_under_nesting() {
    for src in [
        "fnc add(int a,int b): int { return a+b; } pub fnc main(): int { return add(seq(1,2), seq(20,40)); }",
        "fnc id(int a): int { return a; } pub fnc main(): int { return id(seq(1, (> 2, 3), 42)); }",
        "pub fnc main(): int { const int z = seq(let int x = 10, seq(let int y = 2, x * y)); return z + 22; }",
        "fnc id(int a): int { return a; } pub fnc main(): int { let int z = 1; return id(seq z += 1, z * 21); }",
    ] { admitted(src); }

    for src in [
        "pub fnc main(): int { return seq(1,, 42); }",
        "pub fnc main(): int { return seq((1, 2), 42); }",
        "pub fnc main(): int { return seq(let int x = 42, x) + x; }",
        "fnc id(int n): int { return n; } pub fnc main(): int { val n = id(seq let int secret = 42, secret); return secret; }",
        "fnc b(bool v): int { return 42; } pub fnc main(): int { return b(seq(1, 42)); }",
        "pub fnc main(): int { let int n = 1; return seq(n = 2, n += 3); }",
        "pub fnc main(): int { let int n = 1; return seq(n += 1, n += 2,); }",
        "pure fnc p(): int { let int z = 1; return seq(stdio.println(z), z); } pub fnc main(): int { return p(); }",
        "pub fnc main(): int { val int value = 42; return seq(take! value, value); }",
    ] { rejected(src); }
}

#[test]
fn sequence_item_budget_is_enforced_before_unbounded_growth() {
    // Exercise the nested expression parser's global resource admission gate.
    // This is deliberately generated, not hand-written, to cover > MAX_MEMBERS.
    let members = (0..1100).map(|_| "1").collect::<Vec<_>>().join(",");
    let src = format!("pub fnc main(): int {{ return seq({members}); }}");
    let d = Parser::new(lex(&src).unwrap()).parse_program().unwrap_err();
    assert_eq!(d.code, "E_SOURCE_LIMIT", "{d}");
}
