//! End-style callable syntax is parsed by the actual compiler, not a regex stub.
use atto_compiler::compile;

#[test]
fn all_callable_brace_and_end_forms_compile_to_deterministic_wasm() {
    for src in [
        "fnc f(): int { return 5; } pub fnc main(): int { return f(); }",
        "fnc f(): int -> { return 5; } pub fnc main(): int { return f(); }",
        "fnc f() -> int { return 5; } pub fnc main(): int { return f(); }",
        "fnc f() -> int let int x = 3, x + 2 end pub fnc main(): int { return f(); }",
        "fnc f() -> int :: let int x = 3, x + 2 end pub fnc main(): int { return f(); }",
    ] {
        let bytes = compile(src).unwrap_or_else(|d| panic!("{src}: {d}"));
        assert_eq!(&bytes[..8], b"\\0asm\\x01\\0\\0\\0");
        assert_eq!(bytes, compile(src).unwrap(), "compiler output is deterministic");
    }
}

#[test]
fn end_style_does_not_make_commas_valid_in_brace_blocks() {
    let src = "pub fnc main() -> int { let int x = 3, x + 2 }";
    assert_eq!(compile(src).unwrap_err().code, "E_SEMICOLON");
}

#[test]
fn end_style_rejects_missing_end_and_ambiguous_delimiters() {
    assert_eq!(compile("pub fnc main() -> int :: let int x = 3, x + 2")
        .unwrap_err().code, "E_EOF");
    assert_eq!(compile("pub fnc main() -> int :: { return 5; } end")
        .unwrap_err().code, "E_SYNTAX");
    assert_eq!(compile("pub fnc main() -> int :: let int x = 3, x + 2, end")
        .unwrap_err().code, "E_SYNTAX");
}
