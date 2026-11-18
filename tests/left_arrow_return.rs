//! ASCII '<-' as a type-first return delimiter, without changing expression lexing.
use atto_compiler::{ast::Type, compile, compile_with_warnings, lexer::{lex, Tok}, parser::Parser};

const MAIN: &str = "pub fnc main(): int { return 0; }";

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(source).unwrap()).parse_program()
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn fails(source: &str, code: &str) {
    let error = compile(source).expect_err(source);
    assert_eq!(error.code, code, "{source}: {error}");
}

#[test]
fn ascii_and_unicode_prefixes_compile_deterministically() {
    for header in [
        "int• foo()",
        "int <- foo()",
        "int<-foo()",
        "fnc int <- foo()",
    ] {
        let source = format!("{header} {{ return 42; }} {MAIN}");
        let wasm = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(wasm, compile(&source).unwrap(), "{header}");
        assert_eq!(parse(&source).funcs[0].ret, Type::Int);
    }
    // Visibility/purity may affect exports or metadata; verify compilation
    // without requiring the entire Wasm binary to be identical.
    for header in ["pub fnc int <- foo()", "pure int <- foo()"] {
        let source = format!("{header} {{ return 42; }} {MAIN}");
        assert!(compile(&source).unwrap().starts_with(b"\0asm"));
    }
}

#[test]
fn class_actor_and_static_method_headers_use_the_same_return_type() {
    let source = format!(
        "define class Calculator {{ int <- add(a: int,b: int) {{ return a+b; }} }} \
         define class C as static fnc int <- count() {{ return 1; }} end \
         define actor Worker as int <- count() {{ return 1; }} end {MAIN}"
    );
    let program = parse(&source);
    assert_eq!(program.classes[0].methods[0].ret, Type::Int);
    assert_eq!(program.classes[0].methods[0].name, "add");
    assert_eq!(program.classes[1].methods[0].ret, Type::Int);
    assert!(program.classes[1].methods[0].is_static);
    assert_eq!(program.actors[0].methods[0].ret, Type::Int);
}

#[test]
fn nested_builtin_types_and_callable_return_types_parse() {
    for (ty, expected) in [
        ("Option<int>", Type::OptionInt),
        ("Result<int,string>", Type::ResultIntStr),
        ("Option<Result<int,string>>", Type::OptionResultIntStr),
        ("fnc(int)->int", Type::Callable(0)),
    ] {
        let program = parse(&format!("fnc {ty} <- factory() {{ return 0; }}"));
        if matches!(expected, Type::Callable(_)) {
            assert!(matches!(program.funcs[0].ret, Type::Callable(_)));
        } else {
            assert_eq!(program.funcs[0].ret, expected);
        }
    }
}

#[test]
fn redundant_and_contradictory_postfix_types_follow_bullet_rules() {
    for header in ["int <- foo(): int", "fnc int <- foo() -> int"] {
        let source = format!("{header} {{ return 7; }} {MAIN}");
        let (bytes, warnings) = compile_with_warnings(&source).unwrap();
        assert!(bytes.starts_with(b"\0asm"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "W_DUPLICATE_RETURN_TYPE");
    }
    fails("int <- foo(): bool { return 7; }", "E_RETURN_TYPE_CONFLICT");
    fails("int <- foo() -> string { return 7; }", "E_RETURN_TYPE_CONFLICT");
    fails("int <- foo(): !int { return 7; }", "E_RETURN_EXCLUSION");
    fails("int <- foo(): !string { return 7; }", "E_UNSUPPORTED");
    fails("T <- identity<T>(value:T): !T { return value; }", "E_RETURN_EXCLUSION");
}

#[test]
fn left_arrow_is_contiguous_and_only_has_meaning_in_callable_headers() {
    for invalid in [
        "int < - foo() { return 1; }",
        "int < /* gap */ - foo() { return 1; }",
        "int <\n- foo() { return 1; }",
        "int <- () { return 1; }",
        "int <- 123() { return 1; }",
        "int foo() { return 1; }",
    ] {
        assert!(compile(invalid).is_err(), "{invalid}");
    }
    // No new lexer token: ordinary 'x<-1' stays 'x < -1'.
    let ts = lex("fnc foo(): bool { return 1<-2; }").unwrap();
    let lt = ts.iter().position(|t| t.tok == Tok::Lt).unwrap();
    assert_eq!(ts[lt + 1].tok, Tok::Minus);
    compile(&format!("fnc foo(): bool {{ return 1<-2; }} {MAIN}")).unwrap();
}

#[test]
fn leading_ascii_return_type_is_checked_and_warnings_do_not_leak() {
    fails(&format!("int <- foo() {{ return true; }} {MAIN}"), "E_TYPE");
    fails(&format!("bool <- foo() {{ return 42; }} {MAIN}"), "E_TYPE");
    assert_eq!(compile_with_warnings(&format!("int <- foo(): int {{ return 1; }} {MAIN}")).unwrap().1.len(), 1);
    assert!(compile_with_warnings(&format!("int <- foo() {{ return 1; }} {MAIN}")).unwrap().1.is_empty());
}
