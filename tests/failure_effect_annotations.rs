//! Prefix failure annotations and trailing effect clauses must produce the
//! same checked AST and executable guest behavior. Browser scope: core Error.
use atto_compiler::{compile, lexer, parser::Parser};

fn parse(src: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(src).expect("lex")).parse_program().expect("parse")
}

#[test]
fn annotated_function_and_routine_normalize_to_same_effect_contract() {
    let prefix = "@throws(Error)\npub fnc main(): int { return 42; }";
    let suffix = "pub fnc main(): int throws(Error) { return 42; }";
    assert_eq!(parse(prefix).funcs[0].failure_effects.mask(), 1);
    assert_eq!(parse(suffix).funcs[0].failure_effects.mask(), 1);
    // Real compiler guest bytes, not just a parser-only golden comparison.
    assert_eq!(compile(prefix).unwrap(), compile(suffix).unwrap());
    let routine = "@throws(Error)\nroutine answer(): int {return 42;}\npub fnc main(): int throws(Error) {return answer();}";
    assert_eq!(parse(routine).funcs[0].failure_effects.mask(), 1);
    assert!(compile(routine).unwrap().starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn methods_and_actor_receives_share_exact_effect_parser() {
    let scalar = r#"
define class C as
    @throws(Error)
    pub static fnc first(): int { return 40; }
    @raises(Error)
    @panics(Error)
    pub second(int n): int {return n + 2;}
end
@throws(Error)
@raises(Error)
@panics(Error)
pub fnc main(): int {val c = new C(); return c.second(C.first());}
"#;
    let ast = parse(scalar);
    assert_eq!(ast.classes[0].methods[0].failure_effects.mask(), 1);
    assert_eq!(ast.classes[0].methods[1].failure_effects.mask(), 6);
    assert_eq!(ast.funcs[0].failure_effects.mask(), 7);
    assert!(compile(scalar).unwrap().starts_with(b"\0asm\x01\0\0\0"));

    let mtd = r#"
define class C as
    @throws(Error)
    pub mtd add(int n): int {return n + 2;}
end
pub fnc main(): int throws(Error) {val c = new C(); return c.add(40);}
"#;
    assert_eq!(parse(mtd).classes[0].methods[0].failure_effects.mask(), 1);
    assert!(compile(mtd).is_ok());

    let actor = r#"
define actor Worker as
    @throws(Error)
    receive(ActorMail<int> mail): void {}
end
pub fnc main(): int { return 0; }
"#;
    assert_eq!(parse(actor).actors[0].receive.as_ref().unwrap().failure_effects.mask(), 1);
    // An annotated actor receive is *not* a working supervisor transport.
    assert_eq!(compile(actor).unwrap_err().code, "E_UNSUPPORTED");
}

#[test]
fn distinct_prefix_and_suffix_effects_combine_without_double_counting() {
    let src = "@throws(Error)\npub fnc main(): int raises(Error) panics(Error) {return 42;}";
    assert_eq!(parse(src).funcs[0].failure_effects.mask(), 7);
    assert!(compile(src).is_ok());
}

#[test]
fn negative_cases_never_emit_guest_artifacts() {
    for (src, code) in [
        ("@throws(Error) pub fnc main(): int throws(Error) {return 42;}", "E_EFFECT_TYPE"),
        ("@throws(Error) @throws(Error) pub fnc main(): int {return 42;}", "E_EFFECT_TYPE"),
        ("@throws(int) pub fnc main(): int {return 42;}", "E_EFFECT_TYPE"),
        ("@throws(CustomError) pub fnc main(): int {return 42;}", "E_UNSUPPORTED_TYPE"),
        ("@Other(Error) pub fnc main(): int {return 42;}", "E_UNSUPPORTED"),
        ("@throws(Error) define class Foo as final int x: 1; end pub fnc main(): int {return 42;}", "E_EFFECT_TYPE"),
        ("define class Foo as @throws(Error) final int x: 1; end pub fnc main(): int {return 42;}", "E_EFFECT_TYPE"),
        ("@throws(Error) fnc called(): int {return 42;} @raises(Error) pub fnc main(): int {return called();}", "E_EFFECT_TYPE"),
    ] {
        let err = compile(src).expect_err("invalid effect contract must fail closed");
        assert_eq!(err.code, code, "{src}: {err}");
    }
}
