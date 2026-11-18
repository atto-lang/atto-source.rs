//! Comma-delimited `seq` in expression and function-argument positions.
//! Real Rust compiler -> checked AST -> deterministic guest Wasm admission.
use atto_compiler::{
    ast::{ExprKind, Stmt},
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
