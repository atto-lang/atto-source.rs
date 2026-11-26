// Actual compiler admission; the runtime guest harness separately executes the emitted Wasm.
use atto_compiler::{compile, lexer, parser::Parser};

#[test]
fn default_only_procedural_select_uses_normal_checked_blocks() {
    for source in [
        "pub fnc main(): int { do select { default: -> { return 42; } } return 99; }",
        "pub fnc main(): int { do select first over default: { return 42; } end return 99; }",
        "pub fnc main(): int { let n: int = 40; do select fair { default: { n = n + 2; } } return n; }",
        "pub fnc main(): int { do select random over default: { do select { default: { return 42; } } } end return 99; }",
        "pub fnc main(): int { let n: int = 0; loop { do select { default: { n = n + 1; if n < 42 then continue; fi break; } } } return n; }",
    ] {
        let bytes = compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert!(bytes.starts_with(b"\0asm"), "{source}");
    }
}

#[test]
fn unsupported_or_malformed_selects_never_emit_guest_bytes() {
    for source in [
        "pub fnc main(): int { do select {} return 0; }",
        "pub fnc main(): int { return select(3) { default: -> { return 42; } }; }",
        "pub fnc main(): int { return select<int>(3) over default: -> { return 42; } end; }",
        "pub fnc main(): int { return await select<int>(3) { default: -> { return 42; } }; }",
        "pub fnc main(): int { return nb select(3) over default: -> { return 42; } end; }",
        "pub fnc main(): int { do select(3) { default: {} } return 0; }",
        "pub fnc main(): int { return try select(0) { default: -> { return 42; } }; }",
        "pub fnc main(): int { do select over end return 0; }",
        "pub fnc main(): int { do select { default: -> -> {} } return 0; }",
        "pub fnc main(): int { do select over default: => {} end return 0; }",
        "pub fnc main(): int { do select { default: {} default: {} } return 0; }",
        "pub fnc main(): int { do select { when readch ch: val v {} } return 0; }",
        "pub fnc main(): int { do select { default: {} when readch ch: val v {} } return 0; }",
        "pub fnc main(): int { do nb select { default: {} } return 0; }",
        "pub fnc main(): int { nb select { default: {} } return 0; }",
        "pub fnc main(): int { select { default: {} } return 0; }",
        "pub fnc main(): int { val x = select { default: { return 42; } }; return x; }",
        "pub fnc main(): int { do select<int> over default: {} end return 0; }",
        "pub fnc main(): int { do select { default: { val hidden = 42; } } return hidden; }",
        "pub fnc main(): int { do select { default: { return true; } } return 0; }",
        "pub fnc main(): int { do select { default: { break; } } return 0; }",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn default_arm_is_a_lexical_block_not_a_callback() {
    let source = "pub fnc main(): int { do select over default: { return 42; } end return 99; }";
    let p = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    assert!(matches!(&p.funcs[0].body[0], atto_compiler::ast::Stmt::Block(_)));
}
