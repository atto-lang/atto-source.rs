//! First executable slice: typed private Wasm direct functions, no captures.
use atto_compiler::{
    ast::{ExprKind, Type}, check::{self, TExpr, TStmt}, lexer, parser::Parser
};

fn checked(source: &str) -> check::TProgram {
    let p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    check::check(&p).unwrap()
}
#[test]
fn direct_lambda_generates_typed_private_function() {
    let p = checked("pub fnc main(): int { val double = |n| : int -> n * 2; return double(21); }");
    assert_eq!(p.funcs.len(), 2);
    assert_eq!(p.funcs[1].name, "$lambda_0");
    assert_eq!(p.funcs[1].params, [Type::Int]);
    assert_eq!(p.funcs[1].ret, Type::Int);
    let TStmt::Return(Some(value)) = &p.funcs[0].body[0] else {
        panic!("expected direct call return")
    };
    assert!(matches!(&value.e, TExpr::Call(1, args) if args.len() == 1));
}
#[test]
fn explicit_statement_return_compiles_to_typed_function() {
    let source = "pub fnc main(): int { val double = (|n| : int -> return n * 2;); return double(21); }";
    let p = checked(source);
    assert_eq!(p.funcs[1].ret, Type::Int);
    assert!(atto_compiler::compile(source).is_ok());
}
#[test]
fn zero_parameter_lambda_and_deterministic_wasm() {
    let src = "pub fnc main(): int { val answer = || -> 42; return answer(); }";
    let wasm = atto_compiler::compile(src).unwrap();
    assert_eq!(atto_compiler::compile(src).unwrap(), wasm);
    assert!(wasm.starts_with(b"\0asm"));
}
#[test]
fn direct_lambda_captures_immutable_scalar_without_callable_handle() {
    let src = "pub fnc main(): int { val extra = 21; val f = |n| : int -> n + extra; return f(21); }";
    let p = checked(src);
    assert_eq!(p.funcs[1].params, [Type::Int, Type::Int]);
    let TStmt::Return(Some(e)) = &p.funcs[0].body[1] else { panic!("call expected") };
    assert!(matches!(&e.e, TExpr::Call(1, args) if args.len() == 2));
    assert!(atto_compiler::compile(src).is_ok());
}
#[test]
fn direct_lambda_captures_immutable_parameter_and_aliased_parameter() {
    let direct = "fnc calculate(int factor): int { val f = |n| : int -> n * factor; return f(14); } pub fnc main(): int { return calculate(3); }";
    let alias = "fnc calculate(int factor): int { val captured_factor = factor; val f = |n| : int -> n * captured_factor; return f(14); } pub fnc main(): int { return calculate(3); }";
    for src in [direct, alias] {
        let p = checked(src);
        assert_eq!(p.funcs.last().unwrap().params, [Type::Int, Type::Int]);
        assert!(atto_compiler::compile(src).is_ok());
    }
}
#[test]
fn mutable_and_nonscalar_captures_still_fail_closed() {
    let mutable = "pub fnc main(): int { let extra = 1; val f = |n| -> n + extra; return f(1); }";
    assert_eq!(atto_compiler::compile(mutable).unwrap_err().code, "E_CAPTURE");
    let non_scalar = "define class A as int value: 1; end pub fnc main(): int { val obj = new A(); val f = |n| -> n + obj; return f(1); }";
    assert!(atto_compiler::compile(non_scalar).is_err());
}
#[test]
fn escaped_closure_factory_never_masquerades_as_executable() {
    let src = "fnc make_adder(int offset): fnc(int)=>int { return |n| : int -> n + offset; } pub fnc main(): int { return 0; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn lambda_escape_does_not_silently_create_callable() {
    let src = "pub fnc main(): int { val f = |n| -> n + 1; val alias = f; return f(1); }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}
#[test]
fn not_called_lambda_remains_unsupported() {
    let src = "pub fnc main(): int { val f = |n| -> n + 1; return 0; }";
    assert_eq!(atto_compiler::compile(src).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn nested_shadow_does_not_rebind_captured_lexical_value() {
    // Definition-site binding identity is required by Java lexical closures.
    // The current direct Wasm profile lacks an environment object, therefore
    // it must reject a nested shadow instead of passing the wrong value.
    let src = r#"
        pub fnc main(): int {
            val offset = 7;
            val add = |n| : int -> n + offset;
            if true then
                val offset = 99;
                return add(35);
            fi
            return 0;
        }
    "#;
    let d = atto_compiler::compile(src).expect_err("shadow must not retarget capture");
    assert_eq!(d.code, "E_CAPTURE", "{d}");
    assert!(d.message.contains("shadow"), "{d}");
}
