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
    let p = parse(&format!("pub fnc apply(fnc(int)=> int callback, int n): int {{ return n; }} {MAIN}"));
    let Type::Callable(index) = p.funcs[0].params[0].ty else { panic!("expected callable type"); };
    let signature = CallableSignature::from_source_type(
        Type::Callable(index), &p.type_names, p.funcs[0].pos).unwrap();
    assert_eq!(signature.params, [Type::Int]);
    assert_eq!(signature.result, Type::Int);
    assert_eq!(signature.source_spelling(), "fnc(int)=>int");
    assert_eq!(signature.check_arguments(&[Type::Int], p.funcs[0].pos).unwrap(), Type::Int);
}

#[test]
fn identical_source_signatures_are_interned_structurally() {
    let p = parse(&format!(
        "pub fnc compare(fnc(bool, string)=>float f, fnc(boolean, String)=> f64 g): int {{ return 1; }} {MAIN}"
    ));
    let a = p.funcs[0].params[0].ty;
    let b = p.funcs[0].params[1].ty;
    assert_eq!(a, b, "canonical signature equality must not depend on syntax spelling");
    let signature = CallableSignature::from_source_type(a, &p.type_names, p.funcs[0].pos).unwrap();
    assert_eq!(signature.params, [Type::Bool, Type::Str]);
    assert_eq!(signature.result, Type::Float);
    assert_eq!(signature.source_spelling(), "fnc(bool,string)=>float");
    assert_eq!(p.type_names.len(), 1, "the same callable shape is interned once");
}

#[test]
fn zero_parameter_void_callback_is_a_well_formed_source_type() {
    let p = parse(&format!("pub fnc schedule(fnc()=>void callback): int {{ return 1; }} {MAIN}"));
    let signature = CallableSignature::from_source_type(
        p.funcs[0].params[0].ty, &p.type_names, p.funcs[0].pos).unwrap();
    assert!(signature.params.is_empty());
    assert_eq!(signature.result, Type::Void);
}

#[test]
fn callable_return_type_and_annotated_binding_are_retained_by_parser() {
    let f = parse(&format!("pub fnc factory(): fnc(int)=>int {{ return 0; }} {MAIN}"));
    assert!(matches!(f.funcs[0].ret, Type::Callable(_)));
    let b = parse("pub fnc main(): int { val callback: fnc(int)=>int = |n| -> n + 1; return 0; }");
    let atto_compiler::ast::Stmt::Bind { ty: Some(Type::Callable(_)), .. } =
        &b.funcs[0].body[0] else { panic!("typed callable binding was not parsed"); };
}

#[test]
fn callable_value_source_signatures_fail_closed_before_guest_emission() {
    let source = format!("pub fnc apply(fnc(int)=>int callback, int n): int {{ return n; }} {MAIN}");
    let d = err(&source);
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
    assert!(d.message.contains("indirect dispatch"), "{d}");
    assert!(d.pos.line >= 1 && d.pos.col >= 1);
    let source = format!("pub fnc factory(): fnc(int)=>int {{ return 0; }} {MAIN}");
    assert_eq!(err(&source).code, "E_UNSUPPORTED");
    let source = "pub fnc main(): int { val callback: fnc(int)=>int = |n| -> n * 2; return 0; }";
    assert_eq!(err(source).code, "E_UNSUPPORTED");
}

#[test]
fn invalid_parameter_profiles_are_not_silently_accepted() {
    for t in ["fnc(void)=>int", "fnc(Option<int>)=>int",
        "fnc(fnc(int)=>int)=>int", "fnc(int)=>Option<int>",
        "fnc(int)=>fnc(int)=>int"] {
        let src = format!("pub fnc apply({t} cbx): int {{ return 0; }} {MAIN}");
        let d = err(&src);
        assert_eq!(d.code, "E_UNSUPPORTED_TYPE", "{t}: {d}");
    }
}

#[test]
fn callable_parameter_limit_fails_closed() {
    let args = vec!["int"; atto_compiler::parser::MAX_PARAMS + 1].join(",");
    let src = format!("pub fnc apply(fnc({args})=>int callback): int {{ return 0; }} {MAIN}");
    let d = err(&src);
    assert_eq!(d.code, "E_SOURCE_LIMIT", "{d}");
}


#[test]
fn callable_fat_arrow_is_required_in_parameter_return_and_local_type_positions() {
    // Source annotations are type declarations; lambda bodies are runnables.
    for source in [
        "pub fnc use_it(fnc(int) -> int callback): int { return 0; }",
        "pub fnc factory(): fnc(int) -> int { return 0; }",
        "pub fnc main(): int { val callback: fnc(int) -> int = |n| : int -> n; return 0; }",
    ] {
        let e = atto_compiler::compile(source).expect_err("old callable type arrow is invalid");
        assert_eq!(e.code, "E_SYNTAX", "{source}: {e}");
        assert!(e.pos.line > 0 && e.pos.col > 0);
    }
    // Typed lambda runnable arrows retain their original meaning.
    let source = "pub fnc main(): int { val callback = |n| : int -> n * 2; return callback(21); }";
    assert!(atto_compiler::compile(source).is_ok());
}

#[test]
fn canonical_callable_type_factory_compiles_bounded_owned_environment() {
    let source = "fnc make_adder(int offset): fnc(int) => int { return |n| : int -> n + offset; } pub fnc main(): int { return 0; }";
    // This bounded factory escape is now compiled to a private, owned class;
    // general first-class callable parameters remain gated separately.
    let guest = atto_compiler::compile(source).expect("bounded owned closure factory");
    assert!(guest.starts_with(b"\\0asm"));
}

#[test]
fn syntax_requires_fat_arrow_and_result_type() {
    for t in ["fnc(int)", "fnc(int) int", "fnc(int)=>", "fnc(int)->int"] {
        let src = format!("pub fnc apply({t} callback): int {{ return 0; }} {MAIN}");
        assert!(Parser::new(lexer::lex(&src).unwrap()).parse_program().is_err(), "{t}");
    }
}

#[test]
fn direct_ast_cannot_forge_callable_metadata_or_indices() {
    let p = parse(&format!("pub fnc apply(fnc(int)=>int callback): int {{ return 0; }} {MAIN}"));
    let t = p.funcs[0].params[0].ty;
    for forged in ["fnc(bool)=>int", "fnc(void)=>int", "fnc(int)=>nope",
        "fnc(int,)=>int", "fnc(int)=>int trailing"] {
        let mut names = p.type_names.clone();
        names[0].0 = forged.into();
        let result = CallableSignature::from_source_type(t, &names, p.funcs[0].pos);
        if forged == "fnc(bool)=>int" {
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
    let src = "pub fnc main(): int { val callback: fnc(int)=>bool = |n| -> n * 2; return 0; }";
    assert_eq!(err(src).code, "E_TYPE");
}
#[test]
fn annotated_lambda_binding_checks_outer_captures_before_wasm_gate() {
    let src = "pub fnc main(): int { val outer = 7; val callback: fnc(int)=>int = |n| -> n + outer; return 0; }";
    assert_eq!(err(src).code, "E_CAPTURE");
}
#[test]
fn annotated_lambda_binding_checks_arity_before_wasm_gate() {
    let src = "pub fnc main(): int { val callback: fnc(int,int)=>int = |n| -> n * 2; return 0; }";
    assert_eq!(err(src).code, "E_ARITY");
}
#[test]
fn valid_annotated_lambda_binding_is_type_checked_but_not_yet_executable() {
    let src = "pub fnc main(): int { val callback: fnc(int)=>bool = |n| -> n < 4; return 0; }";
    let d = err(src);
    assert_eq!(d.code, "E_UNSUPPORTED");
    assert!(d.message.contains("indirect dispatch"), "{d}");
}
