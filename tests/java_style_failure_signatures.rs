//! Java-style failure clauses and callable body delimiter regression tests.
//! These are executable compiler tests. The current browser backend only
//! resolves core Error; subclass/raise/panic runtime transports are separate.
use atto_compiler::{compile, lexer::lex, parser::Parser};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(src).expect("lex")).parse_program().expect("parse")
}

#[test]
fn java_style_clauses_normalize_with_annotations_and_legacy_clauses() {
    let forms = [
        "pub fnc main() throws Error raises Error -> int\nreturn 42;\nend",
        "pub fnc main() throws Error raises Error -> int { return 42; }",
        "pub fnc main() throws Error raises Error -> int begin return 42; end",
        "pub fnc main() -> int throws Error raises Error begin return 42; end",
        "pub fnc main() -> int throws Error raises Error { return 42; }",
        "pub int• main() throws Error raises Error begin return 42; end",
        "pub int• main() throws Error raises Error { return 42; }",
        "@Throws(Error)\n@Raises(Error)\npub fnc main() -> int begin return 42; end",
        "pub fnc main() -> int throws(Error) raises(Error) { return 42; }",
    ];
    for src in forms {
        let ast = parse(src);
        assert_eq!(ast.funcs.len(), 1, "{src}");
        assert_eq!(ast.funcs[0].failure_effects.mask(), 3, "{src}");
        assert_eq!(ast.funcs[0].ret, atto_compiler::ast::Type::Int, "{src}");
        let wasm = compile(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0", "{src}");
        assert_eq!(wasm, compile(src).unwrap(), "non-deterministic guest for {src}");
    }
}

#[test]
fn class_methods_and_mtd_use_the_same_call_signature_contract() {
    let src = r#"
define class Config as
    pub int• load_config(int n) throws Error raises Error
    begin
        return n;
    end

    pub mtd fallback(int n) throws Error raises Error -> int
        return n;
    end

    get_default() -> int throws Error raises Error {
        return 42;
    }
end
pub fnc main() -> int { return 0; }
"#;
    let ast = parse(src);
    assert_eq!(ast.classes[0].methods.len(), 3);
    for method in &ast.classes[0].methods {
        assert_eq!(method.failure_effects.mask(), 3, "{}", method.name);
        assert_eq!(method.ret, atto_compiler::ast::Type::Int);
    }
    assert!(compile(src).unwrap().starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn inline_effect_delimiter_rules_reject_ambiguous_source() {
    for (src, code) in [
        ("pub fnc main() -> int throws Error return 42; end", "E_SYNTAX"),
        ("pub int• main() throws Error return 42; end", "E_SYNTAX"),
        ("pub fnc main() throws Error -> int return 42; end", "E_SYNTAX"),
        ("pub fnc main() throws int -> int { return 42; }", "E_EFFECT_TYPE"),
        ("pub fnc main() throws IOError -> int { return 42; }", "E_UNSUPPORTED_TYPE"),
        ("@Throws(Error) pub fnc main() throws Error -> int { return 42; }", "E_EFFECT_TYPE"),
        ("pub fnc main() throws Error -> int begin return 42;", "E_EOF"),
        ("pub int• main() throws Error -> int { return 42; }", "E_SYNTAX"),
    ] {
        let err = compile(src).expect_err("invalid signature must not produce wasm");
        assert_eq!(err.code, code, "{src}: {err}");
    }
}

#[test]
fn newline_requirement_survives_comments_and_multiline_effects() {
    let src = "pub fnc main()\n throws Error\n raises Error -> int /* comment */\n return 42;\nend";
    assert_eq!(parse(src).funcs[0].failure_effects.mask(), 3);
    assert!(compile(src).is_ok());

    let same_line = "pub fnc main() throws Error raises Error -> int /* comment */ return 42; end";
    assert_eq!(compile(same_line).unwrap_err().code, "E_SYNTAX");
}
