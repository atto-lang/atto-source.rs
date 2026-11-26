//! Production parser/typed-checker regression; no resource-bound support claim.
use atto_compiler::{compile, lexer::lex, parser::Parser};

#[test]
fn equivalent_failure_headers_preserve_typed_rows_and_emit_wasm() {
    let forms = [
        "pub fnc main() -> int throws Error raises Error begin return 42; end",
        "pub fnc main() throws Error raises Error -> int begin return 42; end",
        "pub fnc main() throws Error raises Error -> int\n return 42;\nend",
        "pub fnc main() throws Error raises Error -> int { return 42; }",
        "pub fnc main() -> int throws Error raises Error { return 42; }",
        "pub fnc main() -> int throws(Error), raises(Error) { return 42; }",
        "pub fnc main() -> int throws Error raises Error ::\n return 42;\nend",
        "pub fnc main() throws Error raises Error -> int // header\n\n return 42;\nend",
    ];
    for src in forms {
        let ast = Parser::new(lex(src).expect("lex")).parse_program().expect(src);
        let rows = ast.funcs[0].declared_effects.as_ref().expect("typed rows");
        assert_eq!(rows.error_types[0], vec![String::from("Error")]);
        assert_eq!(rows.error_types[1], vec![String::from("Error")]);
        assert!(rows.error_types[2].is_empty());
        assert_eq!(ast.funcs[0].ret, atto_compiler::ast::Type::Int);
        let wasm = compile(src).unwrap_or_else(|err| panic!("{src}: {err}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(wasm, compile(src).expect("deterministic compile"));
    }
}

#[test]
fn reject_ambiguous_headers_before_emission() {
    for (src, code) in [
        ("pub fnc main() -> int throws Error return 42; end", "E_BODY_DELIMITER"),
        ("pub fnc main() throws Error -> int return 42; end", "E_BODY_NEWLINE"),
        ("pub fnc main() throws Error : int { return 42; }", "E_RETURN_ANNOTATION"),
        ("pub fnc main() throws Error -> int raises Error { return 42; }", "E_EFFECT_ORDER"),
        ("pub fnc main() -> int throws Error -> int { return 42; }", "E_RETURN_TYPE_CONFLICT"),
        ("pub fnc main() throws Error throws(Error) -> int { return 42; }", "E_DUPLICATE"),
        ("pub fnc main() throws Error, Error -> int { return 42; }", "E_TYPE"),
        ("pub fnc main() throws Unknown -> int { return 42; }", "E_TYPE"),
        ("pub fnc main() -> int Throws(Error) { return 42; }", "E_SYNTAX"),
        ("pub fnc main() throws Error -> int begin return 42;", "E_EOF"),
    ] {
        let err = compile(src).expect_err(src);
        assert_eq!(err.code, code, "{src}: {err}");
    }
}

#[test]
fn structured_resources_never_silently_disappear() {
    for src in [
        "pub fnc main() -> int effects { allocate: 0, suspend: false } budget { stack: 4096, operations: 10000 } { return 42; }",
        "pub fnc main() effects { allocate: 0, suspend: false } budget { stack: 4096, operations: 10000 } -> int { return 42; }",
        "pub fnc main() throws Error budget { stack: 4096 } -> int { return 42; }",
        "pub fnc main() -> int throws Error effects { allocate: 0 } begin return 42; end",
    ] {
        let err = compile(src).expect_err(src);
        assert_eq!(err.code, "E_UNSUPPORTED", "{src}: {err}");
    }
}

#[test]
fn no_contract_legacy_comma_tail_unchanged() {
    let src = "pub fnc main() -> int 42 end";
    assert!(compile(src).expect("legacy comma-tail").starts_with(b"\0asm"));
}
