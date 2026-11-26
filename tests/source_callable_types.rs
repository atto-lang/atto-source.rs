//! Source-level scalar callable type grammar and canonical signature identity.
//! Runtime callable values and Wasm indirect dispatch must continue to reject.
use atto_compiler::{
    ast::Type,
    check::{self, callable_ir::CallableSignature},
    lexer, parser::Parser,
};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(src).expect("lexer"))
        .parse_program().expect("source callable syntax")
}
fn err(src: &str) -> atto_compiler::diag::Diagnostic {
    atto_compiler::compile(src).expect_err("unsupported callable or invalid syntax")
}
const MAIN: &str = "pub fnc main(): int { return 42; }";

#[test]
fn scalar_callable_parameter_is_a_source_type_with_typed_metadata() {
    let p = parse(&format!("pub fnc apply(fnc(int) -> int callback, int n): int {{ return n; }} {MAIN}"));
    let Type::Callable(index) = p.funcs[0].params[0].ty else { panic!("expected callable type"); };
    let signature = CallableSignature::from_source_type(
        Type::Callable(index), &p.type_names, p.funcs[0].pos).unwrap();
    assert_eq!(signature.params, [Type::Int]);
    assert_eq!(signature.result, Type::Int);
    assert_eq!(signature.source_spelling(), "fnc(int)->int");
    assert_eq!(signature.check_arguments(&[Type::Int], p.funcs[0].pos).unwrap(), Type::Int);
}

#[test]
fn identical_source_signatures_are_interned_structurally() {
    let p = parse(&format!(
        "pub fnc compare(fnc(bool, string)->float f, fnc(boolean, String) -> f64 g): int {{ return 1; }} {MAIN}"
    ));
    let a = p.funcs[0].params[0].ty;
    let b = p.funcs[0].params[1].ty;
    assert_eq!(a, b, "canonical signature equality must not depend on syntax spelling");
    let signature = CallableSignature::from_source_type(a, &p.type_names, p.funcs[0].pos).unwrap();
    assert_eq!(signature.params, [Type::Bool, Type::Str]);
    assert_eq!(signature.result, Type::Float);
    assert_eq!(signature.source_spelling(), "fnc(bool,string)->float");
    assert_eq!(p.type_names.len(), 1, "the same callable shape is interned once");
}

#[test]
fn zero_parameter_void_callback_is_a_well_formed_source_type() {
    let p = parse(&format!("pub fnc schedule(fnc()->void callback): int {{ return 1; }} {MAIN}"));
    let signature = CallableSignature::from_source_type(
        p.funcs[0].params[0].ty, &p.type_names, p.funcs[0].pos).unwrap();
    assert!(signature.params.is_empty());
    assert_eq!(signature.result, Type::Void);
}

#[test]
fn callable_return_type_and_annotated_binding_are_retained_by_parser() {
    let f = parse(&format!("pub fnc factory(): fnc(int)->int {{ return 0; }} {MAIN}"));
    assert!(matches!(f.funcs[0].ret, Type::Callable(_)));
    let b = parse("pub fnc main(): int { val callback: fnc(int)->int = |n| -> n + 1; return 0; }");
    let atto_compiler::ast::Stmt::Bind { ty: Some(Type::Callable(_)), .. } =
        &b.funcs[0].body[0] else { panic!("typed callable binding was not parsed"); };
}

#[test]
fn callable_value_source_signatures_fail_closed_before_guest_emission() {
    let source = format!("pub fnc apply(fnc(int)->int callback, int n): int {{ return n; }} {MAIN}");
    let d = err(&source);
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
    assert!(d.message.contains("indirect dispatch"), "{d}");
    assert!(d.pos.line >= 1 && d.pos.col >= 1);
    let source = format!("pub fnc factory(): fnc(int)->int {{ return 0; }} {MAIN}");
    assert_eq!(err(&source).code, "E_UNSUPPORTED");
    let source = "pub fnc main(): int { val callback: fnc(int)->int = |n| -> n * 2; return 0; }";
    assert_eq!(err(source).code, "E_UNSUPPORTED");
}

#[test]
fn invalid_parameter_profiles_are_not_silently_accepted() {
    for t in ["fnc(void)->int", "fnc(Option<int>)->int",
        "fnc(fnc(int)->int)->int", "fnc(int)->Option<int>",
        "fnc(int)->fnc(int)->int"] {
        let src = format!("pub fnc apply({t} cbx): int {{ return 0; }} {MAIN}");
        let d = err(&src);
        assert_eq!(d.code, "E_UNSUPPORTED_TYPE", "{t}: {d}");
    }
}

#[test]
fn callable_parameter_limit_fails_closed() {
    let args = vec!["int"; atto_compiler::parser::MAX_PARAMS + 1].join(",");
    let src = format!("pub fnc apply(fnc({args})->int callback): int {{ return 0; }} {MAIN}");
    let d = err(&src);
    assert_eq!(d.code, "E_SOURCE_LIMIT", "{d}");
}

#[test]
fn syntax_requires_arrow_and_result_type() {
    for t in ["fnc(int)", "fnc(int) int", "fnc(int)->"] {
        let src = format!("pub fnc apply({t} callback): int {{ return 0; }} {MAIN}");
        assert!(Parser::new(lexer::lex(&src).unwrap()).parse_program().is_err(), "{t}");
    }
}

#[test]
fn direct_ast_cannot_forge_callable_metadata_or_indices() {
    let p = parse(&format!("pub fnc apply(fnc(int)->int callback): int {{ return 0; }} {MAIN}"));
    let t = p.funcs[0].params[0].ty;
    for forged in ["fnc(bool)->int", "fnc(void)->int", "fnc(int)->nope",
        "fnc(int,)->int", "fnc(int)->int trailing"] {
        let mut names = p.type_names.clone();
        names[0].0 = forged.into();
        let result = CallableSignature::from_source_type(t, &names, p.funcs[0].pos);
        if forged == "fnc(bool)->int" {
            assert!(result.is_ok(), "this is a well-formed alternate signature");
        } else {
            assert_eq!(result.unwrap_err().code, "E_UNSUPPORTED_TYPE", "{forged}");
        }
    }
    assert_eq!(CallableSignature::from_source_type(Type::Callable(9999),
        &p.type_names, p.funcs[0].pos).unwrap_err().code, "E_UNSUPPORTED_TYPE");
    assert_eq!(CallableSignature::from_source_type(Type::Int,
        &p.type_names, p.funcs[0].pos).unwrap_err().code, "E_TYPE");
}

#[test]
fn established_direct_wasm_lambda_execution_is_unchanged() {
    let src = "pub fnc main(): int { val double = |n| : int -> n * 2; return double(21); }";
    let p = parse(src);
    let checked = check::check(&p).unwrap();
    assert_eq!(checked.funcs.len(), 2);
    assert_eq!(checked.funcs[1].params, [Type::Int]);
    let wasm = atto_compiler::compile(src).unwrap();
    assert_eq!(wasm, atto_compiler::compile(src).unwrap());
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn annotated_lambda_binding_checks_result_before_wasm_gate() {
    let src = "pub fnc main(): int { val callback: fnc(int)->bool = |n| -> n * 2; return 0; }";
    assert_eq!(err(src).code, "E_TYPE");
}
#[test]
fn annotated_lambda_binding_checks_outer_captures_before_wasm_gate() {
    let src = "pub fnc main(): int { val outer = 7; val callback: fnc(int)->int = |n| -> n + outer; return 0; }";
    assert_eq!(err(src).code, "E_CAPTURE");
}
#[test]
fn annotated_lambda_binding_checks_arity_before_wasm_gate() {
    let src = "pub fnc main(): int { val callback: fnc(int,int)->int = |n| -> n * 2; return 0; }";
    assert_eq!(err(src).code, "E_ARITY");
}
#[test]
fn valid_annotated_lambda_binding_is_type_checked_but_not_yet_executable() {
    let src = "pub fnc main(): int { val callback: fnc(int)->bool = |n| -> n < 4; return 0; }";
    let d = err(src);
    assert_eq!(d.code, "E_UNSUPPORTED");
    assert!(d.message.contains("indirect dispatch"), "{d}");
}

#[test]
fn closed_local_invocation_intrinsics_share_direct_wasm_bytes() {
    // Use current-main canonical callable type syntax; typed lambda aliases
    // and type-first parameter syntax are tracked in the older stacked PRs.
    for (sig, lambda, args, tuple) in [
        ("fnc(int,int)->int", "|a,b| : int -> a+b", "20,22", "(20,22)"),
        ("fnc(int,bool)->int", "|a,b| : int -> a+2", "40,true", "(40,true)"),
        ("fnc(int)->int", "|a| : int -> a+2", "40", "(40,)"),
        ("fnc()->int", "|| : int -> 42", "", "()"),
    ] {
        for binding in ["val", "const"] {
            let prefix = format!("pub fnc main():int {{ {binding} f: {sig} = {lambda}; return ");
            let ordinary = format!("{prefix}f({args}); }}");
            let expected = atto_compiler::compile(&ordinary)
                .unwrap_or_else(|e| panic!("{ordinary}: {e}"));
            for expr in [format!("f.call({args})"), format!("f.apply({tuple})")] {
                let src = format!("{prefix}{expr}; }}");
                assert_eq!(expected, atto_compiler::compile(&src)
                    .unwrap_or_else(|e| panic!("{src}: {e}")), "{src}");
            }
        }
    }
}

#[test]
fn intrinsic_uses_existing_checked_capture_snapshot_for_call_only() {
    let prefix = "pub fnc main():int { val x = 2; val f = |n| : int -> n+x; return ";
    let direct = atto_compiler::compile(&format!("{prefix}f(40); }}")).unwrap();
    assert_eq!(direct, atto_compiler::compile(&format!("{prefix}f.call(40); }}")).unwrap());
    assert_eq!(err(&format!("{prefix}f.apply((40,)); }}")).code, "E_UNSUPPORTED");
}

#[test]
fn intrinsic_denials_do_not_materialize_callable_or_tuple_values() {
    let prefix = "pub fnc main():int { val f: fnc(int,int)->int = |a,b| : int -> a+b; return ";
    for (expr, code) in [
        ("f.call(20)", "E_ARITY"),
        ("f.call(0,20,22)", "E_ARITY"),
        ("f.call(20,true)", "E_TYPE"),
        ("f.apply((20,))", "E_ARITY"),
        ("f.apply((20,22,0))", "E_ARITY"),
        ("f.apply((20,true))", "E_TYPE"),
        ("f.apply(20,22)", "E_ARITY"),
        ("f.apply((20))", "E_UNSUPPORTED"),
        ("f.apply([20,22])", "E_UNSUPPORTED"),
        ("f.apply((20+0,22))", "E_UNSUPPORTED"),
        ("f.apply(tuple(20,22))", "E_UNSUPPORTED"),
        ("f.bind(20)", "E_UNSUPPORTED"),
    ] {
        let src = format!("{prefix}{expr}; }}");
        assert_eq!(err(&src).code, code, "{src}");
    }
    // Same spelling as an ordinary user function must not expand its result.
    let src = "fnc tuple(int a,int b): int { return a+b; } pub fnc main():int { val f: fnc(int)->int = |n| : int -> n; return f.apply(tuple(20,22)); }";
    assert_eq!(err(src).code, "E_UNSUPPORTED");
}

#[test]
fn nominal_call_method_remains_ordinary_dispatch() {
    let src = "define class Calculator as pub mtd call(int n):int { return n+1; } end pub fnc main():int { val c = new Calculator(); val f = |n| : int -> n; return f(c.call(41)); }";
    // Force the lexical lambda to infer its argument type at a literal call
    // before supplying a nominal call as a subsequent argument.
    let src = src.replace("return f(c.call(41));", "val initial = f(0); return f(c.call(41))+initial;");
    assert!(atto_compiler::compile(&src).is_ok(), "{src}");
}
