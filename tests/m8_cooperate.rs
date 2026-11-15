//! Canonical browser compiler's narrow M8 actor source -> typed IR -> Wasm.
use atto_compiler::{actor_cooperate, compile, lexer};

const SOURCE: &str = include_str!("fixtures/m8/cooperative_sum.atto");
#[test]
fn canonical_compile_emits_deterministic_m8_guest_wasm() {
    let tok = lexer::lex(SOURCE).unwrap();
    assert!(actor_cooperate::is_candidate(&tok));
    let plan = actor_cooperate::lower(&tok).unwrap();
    assert_eq!(plan.actor, "CooperativeSum");
    assert_eq!((plan.initial_total,plan.initial_index),(0,0));
    assert_eq!(actor_cooperate::IR.len(), 5);
    let code = compile(SOURCE).unwrap();
    assert_eq!(&code[..8], b"\0asm\x01\0\0\0");
    assert_eq!(code, compile(SOURCE).unwrap());
    assert!(code.windows("atto.m8.cooperate".len()).any(|w| w == b"atto.m8.cooperate"));
    assert_eq!(code, actor_cooperate::emit(&plan));
}
#[test]
fn unsupported_effects_and_unsafe_loans_fail_closed() {
    for source in [
        SOURCE.replace("rt cooperate;", "rt borrow total; rt cooperate;"),
        SOURCE.replace("rt cooperate;", "critical-write total; rt cooperate;"),
        SOURCE.replace("rt cooperate;", "await foo; rt cooperate;"),
        SOURCE.replace("rt cooperate;", "select { } rt cooperate;"),
        SOURCE.replace("i = i + 1;", "i = i + 2;"),
        SOURCE.replace("total = total + i;", "total = total + i + i;"),
        SOURCE.replace("let i: int = 0;", "let total: int = 0;"),
        SOURCE.replace("let i: int = 0;", "let i: int = -2;"),
        SOURCE.replace("CooperativeSum", "CooperativeSum$bad"),
        format!("{SOURCE} fnc escape(): int {{return 1;}}"),
    ] {
        let e = compile(&source).expect_err("unsafe/unsupported actor must fail closed");
        assert_eq!(e.code, "E_UNSUPPORTED", "{source}: {e}");
    }
}
#[test]
fn bounded_signed_i64_literals_and_source_limit() {
    let source = SOURCE.replace("let total: int = 0;", "let total: int = 9223372036854775807;");
    assert!(compile(&source).is_ok());
    let source = SOURCE.replace("let total: int = 0;", "let total: int = -9223372036854775808;");
    assert!(compile(&source).is_ok());
    let source = SOURCE.replace("let total: int = 0;", "let total: int = 9223372036854775808;");
    assert!(compile(&source).is_err());
    assert_eq!(compile(&(SOURCE.to_string() + &" ".repeat(65_536))).unwrap_err().code,"E_SOURCE_LIMIT");
}
