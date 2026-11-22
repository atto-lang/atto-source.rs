//! `susp` is the canonical spelling of `suspends`; the effect is never erased.
use atto_compiler::{ast::{ExprKind, Stmt, Type}, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("valid lexing"))
        .parse_program().unwrap_or_else(|e| panic!("{source}: {e}"))
}

#[test]
fn top_level_susp_alias_and_legacy_form_have_the_same_effect() {
    for keyword in ["susp", "suspends"] {
        let p = parse(&format!(
            "pub {keyword} fnc main(): int {{ return await 42; }}"
        ));
        assert!(p.funcs[0].suspends, "effect was lost for {keyword}");
        let guest = atto_compiler::compile(&format!(
            "pub {keyword} fnc main(): int {{ return await 42; }}"
        )).expect("JSPI-enabled compiler should accept both spellings");
        assert!(guest.windows(b"cooperate".len()).any(|x| x == b"cooperate"));
    }
}

#[test]
fn class_methods_static_functions_and_routines_retain_effect() {
    let source = r#"
define class Service as
    susp mtd read(): int { return await 42; }
    static susp fnc connect(): int { return await 42; }
end
pub susp routine worker(): int { return await 42; }
pub fnc main(): int { return 42; }
"#;
    let p = parse(source);
    let methods = &p.classes[0].methods;
    assert_eq!(methods.len(), 2);
    assert!(methods[0].suspends);
    assert!(!methods[0].is_static);
    assert!(methods[1].suspends);
    assert!(methods[1].is_static);
    assert!(p.funcs.iter().any(|f| f.name == "worker" && f.suspends));
}

#[test]
fn suspending_lambda_effect_survives_parsing_but_cannot_be_lowered_as_pure() {
    for keyword in ["susp", "suspends"] {
        let source = format!(
            "pub fnc main(): int {{ val callback = {keyword} |v| -> v + 1; return callback(41); }}"
        );
        let p = parse(&source);
        let Stmt::Bind { init, .. } = &p.funcs[0].body[0] else { panic!("binding required") };
        assert!(matches!(init.kind, ExprKind::Lambda { suspends: true, .. }));
        let diag = atto_compiler::compile(&source).expect_err(
            "suspending lambdas must not be lowered to a pure direct helper"
        );
        assert_eq!(diag.code, "E_UNSUPPORTED", "{diag}");
    }
}

#[test]
fn callable_types_are_effect_distinct_and_aliases_are_canonical() {
    let source = r#"
pub fnc a(fnc(int)->int callback): int { return 1; }
pub fnc b(susp fnc(int)->int callback): int { return 2; }
pub fnc c(suspends fnc(int)->int callback): int { return 3; }
pub fnc main(): int { return 42; }
"#;
    let p = parse(source);
    let types: Vec<Type> = p.funcs.iter().take(3).map(|f| f.params[0].ty).collect();
    assert_ne!(types[0], types[1], "suspending callable must have distinct identity");
    assert_eq!(types[1], types[2], "long and short forms must intern identically");
    assert!(p.type_names.iter().any(|(s,_)| s == "susp fnc(int)->int"));
    assert!(p.type_names.iter().any(|(s,_)| s == "fnc(int)->int"));
    let err = atto_compiler::compile(source).expect_err(
        "an effect-bearing callable must never downgrade to an executable plain callable"
    );
    assert!(matches!(err.code, "E_UNSUPPORTED" | "E_UNSUPPORTED_TYPE"), "{err}");
}

#[test]
fn generator_remains_a_separate_modifier() {
    let p = parse(
        "pub susp fnc *gen numbers(): Generator<int> { yield 42; } pub fnc main(): int { return 42; }"
    );
    assert!(p.funcs[0].suspends);
    assert!(p.funcs[0].is_generator);
}

#[test]
fn duplicate_and_malformed_modifiers_fail_closed() {
    for source in [
        "pub susp susp fnc main(): int { return 42; }",
        "pub suspends susp fnc main(): int { return 42; }",
        "pub susp suspends fnc main(): int { return 42; }",
        "pub fnc main(): int { val x = susp 42; return 42; }",
        "pub fnc suspicious(): int { return 42; } pub susp fnc main(): int { return susp 42; }",
        "pub fnc main(): int { val f: susp int = 42; return 42; }",
    ] {
        assert!(Parser::new(lexer::lex(source).unwrap()).parse_program().is_err(), "{source}");
    }
    let ordinary = parse("pub fnc suspicious(): int { return 42; } pub fnc main(): int { return 42; }");
    assert!(!ordinary.funcs[0].suspends);
}

#[test]
fn purity_and_await_boundaries_are_identical() {
    for form in ["susp", "suspends"] {
        let src = format!("pub pure {form} fnc main(): int {{ return await 42; }}");
        assert_eq!(atto_compiler::compile(&src).unwrap_err().code, "E_PURE");
    }
    assert_eq!(
        atto_compiler::compile("pub fnc main(): int { return await 42; }")
            .unwrap_err().code,
        "E_EFFECT"
    );
}
