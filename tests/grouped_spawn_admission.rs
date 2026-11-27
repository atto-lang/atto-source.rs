//! Production parser fail-closed tests; not actor runtime support.
use atto_compiler::{lexer, parser::Parser};

fn parse(source: &str) -> Result<atto_compiler::ast::Program, atto_compiler::Diagnostic> {
    Parser::new(lexer::lex(source)?).parse_program()
}

#[test]
fn grouped_options_cannot_degrade_to_plain_spawn() {
    for expr in [
        "spawn Worker() with {}",
        "spawn Worker() with { schedule { max_parallelism = 2; affinity = sticky; } }",
        "spawn Worker() with { resources { heap_size = 64 * 1024 * 1024; stack_size = 65536; mailbox_capacity = 1024; } }",
        "spawn Worker() with { init { name = \"worker-1\"; seed = 42; } }",
        "spawn Worker() /* nested /* comment */ ok */ with { init {} }",
        "spawn Worker() with { unknown {} }",
        "spawn Worker() with { init {} init {} }",
    ] {
        let source = format!("pub fnc main(): int {{ let a = {expr}; return 0; }}");
        assert_eq!(parse(&source).unwrap_err().code, "E_SPAWN_OPTIONS_UNSUPPORTED", "{expr}");
    }
}

#[test]
fn legacy_spawn_and_inert_controls_remain_distinct() {
    for source in [
        "pub fnc main(): int { let a = spawn Worker(); return 0; }",
        "pub fnc main(): int { let s = \"spawn Worker() with {}\"; return 0; }",
        "pub fnc main(): int { /* spawn Worker() with {} */ return 0; }",
        "pub fnc main(): int { let with = 42; return with; }",
    ] {
        assert!(parse(source).is_ok(), "{source}");
    }
}

#[test]
fn awaited_spawn_still_fails_closed_at_existing_await_gate() {
    let source = "pub fnc main(): int { let a = await spawn Worker() with {}; return 0; }";
    assert!(parse(source).is_err());
}
