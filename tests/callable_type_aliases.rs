//! Four equivalent callable typedef spellings, grouped inline types, and typed llambdas.
//! Source type acceptance must not accidentally enable first-class Wasm dispatch.
use atto_compiler::{
    ast::{ExprKind, Stmt, Type}, lexer, parser::Parser,
    check::callable_ir::CallableSignature,
};

fn parsed(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(src).expect("lex")).parse_program()
        .unwrap_or_else(|e| panic!("{src}: {e}"))
}
fn bad_parse(src: &str) -> &'static str {
    Parser::new(lexer::lex(src).expect("lex")).parse_program()
        .expect_err("expected syntax/type error").code
}
const DEFINITIONS: &str = r#"
    type F1 = (int, int) => int;
    type F2 = fnc(int, int) => int;
    type F3 = fnc|int, int| => int;
    type F4 = |int, int| => int;
    type F5 = fnc(int, int) -> int;
    type F6 = F1;
"#;

#[test]
fn aliases_are_structurally_interned_across_all_spellings() {
    let src = format!("{DEFINITIONS}
        pub fnc main(): int {{
            const F1 a = |int x, int y| :int -> x + y;
            const F2 b = |x: int, y: int| :int -> x + y;
            const F3 c = |x: int, int y| :int -> x + y;
            const F4 d = |int x, int y| :int -> x + y;
            const F5 e = |x: int, y: int| :int -> x + y;
            const F6 f = |x: int, y: int| :int -> x + y;
            const ((int, int) => int) g = |x: int, y: int| :int -> x + y;
            return 0;
        }}");
    let p = parsed(&src);
    assert_eq!(p.type_names.len(), 1, "one structural signature, not six nominal types");
    let mut ty = None;
    for stmt in &p.funcs[0].body[..7] {
        let Stmt::Bind { ty: Some(t), init, .. } = stmt else { panic!("typed binding"); };
        assert!(matches!(init.kind, ExprKind::Lambda { .. }));
        assert!(matches!(t, Type::Callable(_)));
        if let Some(previous) = ty { assert_eq!(*t, previous); }
        ty = Some(*t);
    }
    let signature = CallableSignature::from_source_type(
        ty.unwrap(), &p.type_names, p.funcs[0].pos).unwrap();
    assert_eq!(signature.params, [Type::Int, Type::Int]);
    assert_eq!(signature.result, Type::Int);
    assert_eq!(signature.source_spelling(), "fnc(int,int)->int");
}

#[test]
fn lambda_parameter_annotations_are_preserved_not_discarded() {
    let p = parsed("pub fnc main(): int {
        val f = |int a, b: float| :float -> a + b;
        return 0;
    }");
    let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else { panic!("binding"); };
    let ExprKind::Lambda { params, param_types, .. } = &init.kind else { panic!("lambda"); };
    assert_eq!(params, &["a", "b"]);
    assert_eq!(param_types, &[Some(Type::Int), Some(Type::Float)]);
}

#[test]
fn mismatching_lambda_annotations_fail_before_wasm_gate() {
    for (parameter, expected) in [
        ("|a: bool, b: int| :int -> b", "E_TYPE"),
        ("|int a| :int -> a", "E_ARITY"),
        ("|int a, int b| :bool -> a + b", "E_TYPE"),
    ] {
        let src = format!("type Binary = (int,int)=>int;
            pub fnc main(): int {{ const Binary f = {parameter}; return 0; }}");
        let d = atto_compiler::compile(&src).expect_err("invalid callable must reject");
        assert_eq!(d.code, expected, "{parameter}: {d}");
    }
}

#[test]
fn closed_typed_callable_uses_verified_static_dispatch() {
    let source = format!("{DEFINITIONS}
        pub fnc main(): int {{
            const F1 f = |int a, int b| :int -> a + b;
            return f(20, 22);
        }}");
    let wasm = atto_compiler::compile(&source).expect("static callable execution");
    assert!(wasm.starts_with(b"\0asm"));
}
#[test]
fn escaping_typed_callable_still_fails_closed_without_indirect_dispatch() {
    let source = format!("{DEFINITIONS}
        pub fnc main(): int {{
            const F1 f = |int a, int b| :int -> a + b;
            val escaped = f;
            return 0;
        }}");
    let d = atto_compiler::compile(&source).expect_err("no callable value storage ABI");
    assert_eq!(d.code, "E_UNSUPPORTED", "{d}");
}

#[test]
fn malformed_and_unsupported_signatures_reject() {
    for source in [
        "type F = (int, void) => int;",
        "type F = (int) => Option<int>;",
        "type F = (int,int);",
        "type F = fnc|int,int| -> int;",
        "type F = (int,)=>int;",
        "type F = (int,int)=>int; type F = (bool)=>int;",
        "type F = (int,int)=>int; type int = F;",
        "type F = (int,int)=>int; type B = Missing;",
    ] {
        let code = bad_parse(source);
        assert!(matches!(code, "E_UNSUPPORTED_TYPE" | "E_SYNTAX" | "E_DUPLICATE"
            | "E_SEMICOLON"), "{source}: {code}");
    }
}

#[test]
fn existing_direct_lambda_wasm_path_remains_executable() {
    let source = "pub fnc main(): int {
        val add = |int a, b: int| :int -> a + b;
        return add(20, 22);
    }";
    let output = atto_compiler::compile(source).unwrap();
    assert!(output.starts_with(b"\0asm"));
}
