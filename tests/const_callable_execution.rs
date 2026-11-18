//! Closed const callable values become checked direct calls without a runtime
//! function table, closure environment, mutable reference or forgeable handle.
use atto_compiler::{ast::{ExprKind, Stmt, Type}, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap()
}
fn rejects(source: &str, expected: &str) {
    let d = atto_compiler::compile(source).expect_err("must not emit a guest");
    assert_eq!(d.code, expected, "{source}: {d}");
}

#[test]
fn four_global_signature_spellings_generate_checked_direct_functions() {
    for signature in [
        "(int, int) => int",
        "fnc(int, int) => int",
        "fnc|int, int| => int",
        "|int, int| => int",
        "fnc(int, int) -> int",
    ] {
        let source = format!(
            "type Binary = {signature};
             const Binary calculate = |a: int, b: int| :int -> a + b;
             pub fnc main(): int {{ return calculate(20, 22); }}"
        );
        let p = parse(&source);
        assert_eq!(p.funcs.len(), 2);
        assert_eq!(p.funcs[0].name, "calculate");
        assert_eq!(p.funcs[0].ret, Type::Int);
        assert_eq!(p.funcs[0].params.iter().map(|p| p.ty).collect::<Vec<_>>(),
                   [Type::Int, Type::Int]);
        let guest = atto_compiler::compile(&source).unwrap_or_else(|e| panic!("{signature}: {e}"));
        assert!(guest.starts_with(b"\0asm"));
        assert_eq!(guest, atto_compiler::compile(&source).unwrap(), "deterministic Wasm");
    }
}

#[test]
fn inline_and_zero_arity_global_signatures_compile() {
    for (signature, lambda, call) in [
        ("((int,int)=>int)", "|int a, int b| :int -> a+b", "add(20,22)"),
        ("(|| => int)", "|| :int -> 42", "add()"),
        ("(fnc()=>int)", "|| :int -> 42", "add()"),
    ] {
        let source = format!("const {signature} add = {lambda}; pub fnc main():int {{ return {call}; }}");
        assert!(atto_compiler::compile(&source).is_ok(), "{source}");
    }
}

#[test]
fn closed_local_const_calls_are_statically_lowered_even_with_nonliteral_arguments() {
    let source = r#"
      type Binary = (int,int)=>int;
      fnc calculate(int x):int {
        const Binary add = |int a, b:int| :int -> a+b;
        return add(x,22);
      }
      pub fnc main():int { return calculate(20); }
    "#;
    let p = parse(source);
    let Stmt::Bind { ty: Some(Type::Callable(_)), init, .. } = &p.funcs[0].body[0]
        else { panic!("typed local constant not parsed"); };
    assert!(matches!(init.kind, ExprKind::Lambda { .. }));
    assert!(atto_compiler::compile(source).is_ok());
}

#[test]
fn unused_typed_closed_binding_needs_no_runtime_environment() {
    let source = "type F = (int)=>int; pub fnc main():int { const F f = |int n| :int -> n+1; return 42; }";
    assert!(atto_compiler::compile(source).is_ok());
}

#[test]
fn global_rejects_wrong_return_wrong_arity_and_unsafe_capture() {
    rejects("type F=(int,int)=>int; const F f=|a:bool,b:int| :int -> b; pub fnc main():int{return 0;}", "E_TYPE");
    rejects("type F=(int,int)=>int; const F f=|a:int| :int -> a; pub fnc main():int{return 0;}", "E_ARITY");
    rejects("type F=(int,int)=>int; const F f=|a:int,b:int| :bool -> true; pub fnc main():int{return 0;}", "E_TYPE");
    rejects("type F=(int)=>int; const F f=|a:int| :int -> a+outside; pub fnc main():int{return 0;}", "E_CAPTURE");
}

#[test]
fn global_const_identity_conflicts_fail_closed_regardless_of_arity_and_order() {
    rejects("type F=(int)=>int; const F f=|x:int| :int -> x; const F f=|x:int| :int -> x; pub fnc main():int{return 0;}", "E_DUPLICATE");
    rejects("type F=(int)=>int; const F f=|x:int| :int -> x; fnc f():int{return 0;} pub fnc main():int{return 0;}", "E_DUPLICATE");
    rejects("fnc f():int{return 0;} type F=(int)=>int; const F f=|x:int| :int -> x; pub fnc main():int{return 0;}", "E_DUPLICATE");
    rejects("type F=(int)=>int; const F F=|x:int| :int -> x; pub fnc main():int{return 0;}", "E_DUPLICATE");
}

#[test]
fn cannot_rebind_escape_or_create_mutable_callable_storage() {
    rejects("type F=(int)=>int; pub fnc main():int { let F f = |x:int| :int -> x; return 0; }", "E_UNSUPPORTED");
    rejects("type F=(int)=>int; pub fnc main():int { const F f=|x:int| :int -> x; f=|y:int| :int -> y; return 0; }", "E_IMMUTABLE");
    rejects("type F=(int)=>int; pub fnc main():int { const F f=|x:int| :int -> x; val alias = f; return 0; }", "E_UNSUPPORTED");
    rejects("type F=(int)=>int; pub fnc main():int { const F f=|x:int| :int -> x; return f; }", "E_UNSUPPORTED");
}

#[test]
fn unsupported_file_const_forms_fail_without_runtime_storage() {
    rejects("const int n = 42; pub fnc main():int {return 0;}", "E_UNSUPPORTED_TYPE");
    rejects("type F=(int)=>int; const F f = 42; pub fnc main():int {return 0;}", "E_UNSUPPORTED_TYPE");
    rejects("type F=(int)=>int; pub fnc main():int { const F f=|x:int| :int -> x; return f(1,2); }", "E_ARITY");
    rejects("type F=(int)=>int; pub fnc main():int { const F f=|x:int| :int -> x; return f(true); }", "E_TYPE");
}
