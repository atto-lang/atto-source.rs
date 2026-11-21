//! Stack-safety boundary checks for syntax-only lambda declarations.
//! Deeply nested types/lambdas must return a diagnostic, never abort rustc.

use atto_compiler::{lexer, parser::Parser};

fn parse(expr: &str) -> Result<atto_compiler::ast::Program, atto_compiler::Diagnostic> {
    let source = format!("pub fnc main(): int {{ val f = {expr}; return 0; }}");
    Parser::new(lexer::lex(&source)?).parse_program()
}

#[test]
fn nested_type_annotations_are_bounded_before_stack_exhaustion() {
    for depth in [60, 150] {
        let mut ty = "int".to_string();
        for _ in 0..depth { ty = format!("Box<{ty}>"); }
        for expr in [
            format!("|x| : {ty} -> x"),
            format!("|x| -> {ty} {{ return x; }}"),
        ] {
            let diagnostic = parse(&expr).expect_err("deep lambda type must reject");
            assert_eq!(diagnostic.code, "E_NESTING", "depth={depth}: {diagnostic}");
        }
    }
}

#[test]
fn nested_lambda_bodies_are_bounded_before_stack_exhaustion() {
    for depth in [40, 150] {
        let expr = format!("{}1", "|x| -> ".repeat(depth));
        let diagnostic = parse(&expr).expect_err("deep lambdas must reject");
        assert_eq!(diagnostic.code, "E_NESTING", "depth={depth}: {diagnostic}");
    }
}

#[test]
fn ordinary_lambda_annotations_and_nested_expressions_still_parse() {
    for expr in [
        "|x| : Foo<Bar<Baz>> -> x",
        "|x| -> |y| : int -> x + y",
        "|x| -> |y| -> |z| -> x + y + z",
    ] {
        parse(expr).unwrap_or_else(|e| panic!("{expr}: {e}"));
    }
}
