//! Production Rust/Wasm fail-closed boundary for proposed structural `~` shorthand.
//! These are *negative* compiler tests; they do not prove structural lowering.
//! A separate target grammar contract lives in atto-lang/atto-spec.
use atto_compiler::{compile, lexer::{lex, Tok}};

#[test]
fn tilde_tokenization_is_explicit_and_comment_string_safe() {
    let tokens = lex("~Point ~ Point // ~ignore\n\"~literal\" /* ~ignore */").unwrap();
    assert!(matches!(tokens[0].tok, Tok::Tilde));
    assert!(matches!(&tokens[1].tok, Tok::Ident(s) if s == "Point"));
    assert!(matches!(tokens[2].tok, Tok::Tilde));
    assert!(matches!(&tokens[3].tok, Tok::Ident(s) if s == "Point"));
    assert!(matches!(&tokens[4].tok, Tok::Str(s) if s == "~literal"));
    assert!(matches!(tokens[5].tok, Tok::Eof));
}

#[test]
fn unsupported_structural_formals_and_type_positions_produce_no_wasm() {
    for (src, code) in [
        ("fnc main(~Point p): int { return 42; }", "E_UNSUPPORTED"),
        ("fnc main(structural Point p): int { return 42; }", "E_UNSUPPORTED"),
        ("fnc main(p: ~Point): int { return 42; }", "E_UNSUPPORTED"),
        ("fnc main(p: structural Point): int { return 42; }", "E_UNSUPPORTED"),
        ("routine main(~Point p): int { return 42; }", "E_UNSUPPORTED"),
        ("routine main(p: ~{x: int}): int { return 42; }", "E_UNSUPPORTED"),
        ("~ fnc main(): int { return 42; }", "E_UNSUPPORTED"),
        ("fnc main(): ~Point { return 42; }", "E_UNSUPPORTED"),
        ("fnc main(): int { val p: ~Point = 1; return 42; }", "E_UNSUPPORTED"),
        ("define class C as end mtd (self: C) read(x: ~Point) -> int :: 42 end pub fnc main(): int { return 42; }", "E_UNSUPPORTED_TYPE"),
        ("define class C as end mtd (self: C) read(x: structural Point) -> int :: 42 end pub fnc main(): int { return 42; }", "E_UNSUPPORTED_TYPE"),
    ] {
        let diagnostic = compile(src).expect_err(src);
        assert_eq!(diagnostic.code, code, "{src}: {diagnostic}");
    }
}

#[test]
fn structural_patterns_and_projections_cannot_fall_through_to_value_matching() {
    for src in [
        "fnc main(): int { match 1 over on ~Point -> { return 42; } on _ -> { return 0; } end }",
        "fnc main(): int { match 1 over on is ~Point -> { return 42; } on _ -> { return 0; } end }",
        "fnc main(): int { return ~candidate; }",
        "fnc main(): int { return structural candidate; }",
        "fnc main(): int { return ~(field: 42); }",
        "fnc main(): int { return ~(key => 42); }",
        "fnc main(): int { return ~(); }",
        "fnc main(): int { return ~ (candidate); }",
    ] {
        let diagnostic = compile(src).expect_err(src);
        assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{src}: {diagnostic}");
    }
}

#[test]
fn colon_labels_remain_distinct_from_structural_and_aggregate_forms() {
    for src in [
        "fnc main(): int { return f(point: ~candidate); }",
        "fnc main(): int { return f(1, point: ~candidate); }",
        "fnc main(): int { return f(point: 1, 2); }",
        "fnc main(): int { return f(point: ~(x: 1)); }",
        "fnc main(): int { return f(1, point: 2); }",
    ] {
        let diagnostic = compile(src).expect_err(src);
        assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{src}: {diagnostic}");
    }
}

#[test]
fn ordinary_positional_calls_are_not_regressed() {
    let src = "fnc sum(int a, int b): int { return a + b; } pub fnc main(): int { return sum(20, 22); }";
    let artifact = compile(src).expect("ordinary positional calls remain executable");
    assert_eq!(&artifact[..8], b"\0asm\x01\0\0\0");
    assert_eq!(artifact, compile(src).unwrap(), "output remains deterministic");
}
