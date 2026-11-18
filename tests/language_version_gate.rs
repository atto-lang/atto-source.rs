//! Language-versioned callable body compatibility. Legacy v1 is the
//! production API default; strict :: body selection is explicitly opt-in.
use atto_compiler::{
    compile, compile_with_version,
    lexer::lex,
    parser::{LanguageVersion, Parser},
    ast::Stmt,
};

fn good(source: &str, version: LanguageVersion) -> Vec<u8> {
    let bytes = compile_with_version(source, version)
        .unwrap_or_else(|e| panic!("{version:?}: {source}\n{e}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(bytes, compile_with_version(source, version).unwrap());
    bytes
}
fn bad(source: &str, version: LanguageVersion, code: &str) {
    let err = compile_with_version(source, version).expect_err("expected rejection");
    assert_eq!(err.code, code, "{source}: {err}");
}

const UNMARKED_COMMA: &str = "fnc f() -> int const z: int = 40, z + 2 end pub fnc main(): int { return f(); }";
const MARKED_COMMA: &str = "fnc f() -> int :: const z: int = 40, z + 2 end pub fnc main(): int { return f(); }";
const UNMARKED_SEMI: &str = "fnc f() -> int const z: int = 40; return z + 2; end pub fnc main(): int { return f(); }";

#[test]
fn legacy_default_preserves_optional_coloncolon_and_tail_returns() {
    let a = good(UNMARKED_COMMA, LanguageVersion::Legacy);
    assert_eq!(a, compile(UNMARKED_COMMA).unwrap(), "legacy is default API version");
    assert_eq!(a, good(MARKED_COMMA, LanguageVersion::Legacy));
    good(UNMARKED_SEMI, LanguageVersion::Legacy);
    good("fnc f() -> int 42 end pub fnc main(): int { return f(); }", LanguageVersion::Legacy);
}

#[test]
fn explicit_version_requires_marker_for_commas_but_keeps_statements() {
    bad(UNMARKED_COMMA, LanguageVersion::Explicit, "E_BODY_VERSION");
    good(MARKED_COMMA, LanguageVersion::Explicit);
    good(UNMARKED_SEMI, LanguageVersion::Explicit);
    let no_tail = "fnc f() -> int 42 end pub fnc main(): int { return f(); }";
    assert!(compile_with_version(no_tail, LanguageVersion::Explicit).is_err(),
        "unmarked v2 expression must never acquire implicit return");
}

#[test]
fn migration_diagnostic_identifies_both_safe_manual_choices() {
    let error = compile_with_version(UNMARKED_COMMA, LanguageVersion::Explicit).unwrap_err();
    assert_eq!(error.code, "E_BODY_VERSION");
    assert_eq!(error.pos.line, 1);
    assert!(error.pos.col > 1);
    assert!(error.message.contains("'::'"), "{error}");
    assert!(error.message.contains("';' statements"), "{error}");
    assert!(error.message.contains("return/^"), "{error}");
}

#[test]
fn nested_argument_commas_do_not_switch_legacy_statement_mode() {
    let src = "fnc add(int a,int b): int { return a+b; } fnc f() -> int const z: int = add(20,22); return z; end pub fnc main(): int { return f(); }";
    good(src, LanguageVersion::Legacy);
    good(src, LanguageVersion::Explicit);
    // Unmarked tail with nested call comma is still a v1 comma/tail body.
    let src = "fnc add(int a,int b): int { return a+b; } fnc f() -> int add(20,22) end pub fnc main(): int { return f(); }";
    good(src, LanguageVersion::Legacy);
    assert!(compile_with_version(src, LanguageVersion::Explicit).is_err());
}

#[test]
fn version_selection_is_shared_by_routines_and_instance_methods() {
    let routine = "routine f() -> int const n: int = 41, n + 1 end pub fnc main(): int { return f(); }";
    good(routine, LanguageVersion::Legacy);
    bad(routine, LanguageVersion::Explicit, "E_BODY_VERSION");
    let class = "define class Foo as pub answer() -> int const n: int = 41, n + 1 end end pub fnc main(): int { const f = new Foo(); return f.answer(); }";
    good(class, LanguageVersion::Legacy);
    bad(class, LanguageVersion::Explicit, "E_BODY_VERSION");
    let marked = "define class Foo as pub answer() -> int :: const n: int = 41, n + 1 end end pub fnc main(): int { const f = new Foo(); return f.answer(); }";
    good(marked, LanguageVersion::Explicit);
}

#[test]
fn implicit_tail_is_lexically_owned_by_version_not_inferred_from_checker() {
    let tokens = lex(UNMARKED_COMMA).unwrap();
    let p = Parser::with_version(tokens.clone(), LanguageVersion::Legacy).parse_program().unwrap();
    assert!(matches!(p.funcs[0].body.last(), Some(Stmt::Return(Some(_),_))));
    assert!(Parser::with_version(tokens, LanguageVersion::Explicit).parse_program().is_err());
}

#[test]
fn nested_sequences_and_comma_return_positions_are_orthogonal() {
    let source = "fnc f() -> int :: const n: int = seq(1, 42), n end pub fnc main(): int { return 1, f(); }";
    good(source, LanguageVersion::Legacy);
    good(source, LanguageVersion::Explicit);
    let source = "pub fnc main(): int { let int z = 0; return seq(z += 2, z * 21); }";
    assert_eq!(good(source, LanguageVersion::Legacy), good(source, LanguageVersion::Explicit));
}

#[test]
fn version_values_are_closed_and_do_not_guess_unsupported_editions() {
    assert_eq!(LanguageVersion::from_number(1), Some(LanguageVersion::Legacy));
    assert_eq!(LanguageVersion::from_number(2), Some(LanguageVersion::Explicit));
    for value in [0, 3, 2026, u32::MAX] {
        assert_eq!(LanguageVersion::from_number(value), None);
    }
}
