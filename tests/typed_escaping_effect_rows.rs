//! Joined #275/#296: source failure rows are typed, explicit upper bounds,
//! while trap transports original guest Error identity via Result.
use atto_compiler::{compile, lexer::lex, parser::Parser, ast::Program};

fn parse(src: &str) -> Program {
    Parser::new(lex(src).expect("lexer")).parse_program().expect("parser")
}
fn good(src: &str) {
    let wasm = compile(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    assert_eq!(wasm, compile(src).unwrap());
}
fn bad(src: &str, code: &str) {
    let error = compile(src).expect_err("invalid effect signature emitted guest bytes");
    assert_eq!(error.code, code, "{error}\n{src}");
}
#[test]
fn explicit_empty_and_nominal_names_survive_ast() {
    let p = parse("@Throws(Error)\n@Raises()\n@Panics(Error)\npub fnc main(): int { return 42; }");
    let d = p.funcs[0].declared_effects.as_ref().unwrap();
    assert_eq!(d.declared, [true, true, true]);
    assert_eq!(d.error_types[0], vec!["Error"]);
    assert!(d.error_types[1].is_empty());
    assert_eq!(d.error_types[2], vec!["Error"]);
    assert!(d.can_throw() && !d.can_raise());
    good("@Throws()\npub fnc main(): int { return 42; }");
    good("@throws()\npub fnc main(): int { return 42; }");
    good("pub fnc main(): int throws() { return 42; }");
    good("@Throws()\n@Raises()\npub fnc main(): int { return 42; }");
    good("@Throws()\npub fnc main(): int { try { throw Error(\"handled\"); } catch (Error e) { return 42; } }");
}
#[test]
fn empty_bound_rejects_reachable_throw_and_preserves_trap_discharging() {
    bad("@Throws()\npub fnc main(): int { throw Error(\"escape\"); }", "E_EFFECT");
    good("fnc fail(): int throws(Error) { throw Error(\"caught\"); }\n@Throws()\npub fnc main(): int { val Result<int,Error> x = trap fail(); return 42; }");
    good("@Throws()\npub fnc main(): int { return 42; throw Error(\"unreachable\"); }");
    bad("fnc fail(): int throws(Error) { throw Error(\"escape\"); }\n@Throws()\npub fnc main(): int { return fail(); }", "E_EFFECT");
}
#[test]
fn trap_cannot_discharged_raise_and_unknown_types_fail_closed() {
    bad("fnc fatal(): int raises(Error) { raise Error(\"escalate\"); }\n@Throws()\npub fnc main(): int { val Result<int,Error> x = trap fatal(); return 42; }", "E_EFFECT");
    bad("@Throws(CustomError)\npub fnc main(): int { return 42; }", "E_TYPE");
    bad("@Throws(Error,Error)\npub fnc main(): int { return 42; }", "E_DUPLICATE");
    bad("@Throws(Error,)\npub fnc main(): int { return 42; }", "E_TYPE");
    bad("@Throws()\n@Throws(Error)\npub fnc main(): int { return 42; }", "E_DUPLICATE");
    bad("@Throws()\npub fnc main(): int throws(Error) { return 42; }", "E_DUPLICATE");
    // Fatal panic-free annotations are not sound until runtime/actor/heap
    // paths are modeled. Fail closed rather than silently assert zero panic.
    bad("@Panics()\npub fnc main(): int { return 42; }", "E_UNSUPPORTED_EFFECT");
    bad("pub fnc main(): int panics() { return 42; }", "E_UNSUPPORTED_EFFECT");
}
#[test]
fn negative_unsupported_annotations_do_not_emit_guest() {
    bad("@Purity()\npub fnc main(): int { return 42; }", "E_UNSUPPORTED");
    bad("pub fnc main(): int Throws(Error) { return 42; }", "E_SYNTAX");
}
