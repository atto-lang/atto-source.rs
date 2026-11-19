//! Real compiler contracts for the corrected exact website boolean preview.
use atto_compiler::{compile, lexer::{lex, Tok}};
fn valid(src: &str) {
    let bytes = compile(src).unwrap_or_else(|e| panic!("expected valid Wasm: {e}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(src).unwrap());
}
fn invalid(src: &str) {
    let d = compile(src).expect_err("invalid boolean source must emit no Wasm");
    assert!(d.code.starts_with("E_"), "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0, "{d}");
}
#[test]
fn corrected_exact_website_boolean_source_compiles_guest_deterministically() {
    valid(include_str!("playground_previews/boolean-operators.atto"));
}
#[test]
fn infix_short_circuit_uses_typed_ternary_and_does_not_break_lambda_pipes() {
    valid(r#"
        pub fnc main(): int {
            val a = true; val b = false;
            val ok = a && b || a;
            stdio.println(ok);
            return 42;
        }
    "#);
    let tokens = lex("|| -> 42").unwrap();
    assert_eq!(tokens[0].tok, Tok::Pipe);
    assert_eq!(tokens[1].tok, Tok::Pipe);
    valid("pub fnc main(): int { val f = || -> 42; return f(); }");
}
#[test]
fn invalid_operator_types_and_retired_xor_remain_rejected() {
    invalid("pub fnc main(): int { stdio.println(1 && true); return 0; }");
    invalid("pub fnc main(): int { stdio.println(true || 7); return 0; }");
    invalid("pub fnc main(): int { stdio.println(true ^^ false); return 0; }");
    invalid("pub fnc main(): int { stdio.println(And(true)); return 0; }");
    invalid("pub fnc main(): int { stdio.println(And(true, 7)); return 0; }");
    invalid("pub fnc main(): int { stdio.println(Or([])); return 0; }");
}
#[test]
fn literal_bool_array_packs_cannot_become_general_array_runtime() {
    valid("pub fnc main(): int { stdio.println(And([true, true, false])); return 0; }");
    invalid("pub fnc main(): int { val list = [true,false]; return 0; }");
    invalid(r#"
        fnc effect(): bool { return true; }
        pub fnc main(): int { stdio.println(And([effect(), false])); return 0; }
    "#);
}
