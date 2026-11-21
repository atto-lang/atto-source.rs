//! Compiler-backed typeof return queries for the scalar AOT browser profile.
//! This slice is deliberately limited to parameter names; forward locals and
//! callable signature queries remain unsupported rather than runtime-reflective.
use atto_compiler::{ast::Type, compile, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("lexer"))
        .parse_program().expect("static typeof parser")
}

fn main_with(query: &str) -> String {
    format!("pub fnc identity(int x): {query} {{ return x; }} pub fnc main(): int {{ return identity(5); }}")
}

#[test]
fn bare_and_parenthesized_parameter_queries_have_identical_typed_ast() {
    let bare = parse(&main_with("typeof x"));
    let parenthesized = parse(&main_with("typeof(x)"));
    assert_eq!(bare.funcs[0].ret, Type::Int);
    assert_eq!(bare.funcs[0].ret, parenthesized.funcs[0].ret);
    assert_eq!(bare.funcs[0].params[0].ty, parenthesized.funcs[0].params[0].ty);
}

#[test]
fn both_spelling_variants_emit_deterministic_genuine_wasm() {
    for query in ["typeof x", "typeof(x)"] {
        let source = main_with(query);
        let wasm = compile(&source).unwrap_or_else(|e| panic!("{query}: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{query}: not a Wasm guest");
        assert_eq!(wasm, compile(&source).unwrap(), "{query}: nondeterministic AOT lowering");
    }
}

#[test]
fn typeof_bang_does_not_create_a_new_operator() {
    for query in ["typeof! x", "typeof !x"] {
        let error = compile(&main_with(query)).expect_err("bang query must fail");
        assert_eq!(error.code, "E_TYPE_QUERY_UNSUPPORTED", "{query}: {error}");
    }
}

#[test]
fn unbound_and_expression_queries_fail_without_a_wasm_artifact() {
    let unbound = compile(&main_with("typeof missing")).expect_err("unbound typeof");
    assert_eq!(unbound.code, "E_TYPE_QUERY_UNBOUND");
    for query in ["typeof()", "typeof(x, x)", "typeof(x + 1)", "typeof(make())"] {
        compile(&main_with(query)).expect_err(query);
    }
}

#[test]
fn no_implicit_runtime_typeof_expression_is_introduced() {
    let source = "pub fnc main(): int { val int x = 5; return typeof x; }";
    compile(source).expect_err("typeof in value position must remain invalid");
}
