//! U+2022 return-type delimiter through the production frontend.
//! The browser subset intentionally does not claim generic code generation.
use atto_compiler::{ast::Type, compile, compile_with_warnings, lexer::lex, parser::Parser};

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
fn all_top_level_spellings_emit_deterministic_wasm() {
    for header in [
        "fnc f(): int",
        "fnc int• f()",
        "int• f()",
        "fnc int • f()",
        "pub fnc int• f()",
        "pure int• f()",
    ] {
        let source = format!("{header} {{ return 42; }} pub fnc main(): int {{ return f(); }}");
        let wasm = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(wasm, compile(&source).unwrap());
    }
}

#[test]
fn class_brace_and_as_end_bodies_accept_bullet_methods() {
    for class in [
        "define class Calculator { pub int• add(a: int, b: int) { return a+b; } }",
        "define class Calculator as pub int• add(a: int, b: int) { return a+b; } end",
        "define class Calculator as { pub fnc int• add(a: int, b: int) { return a+b; } }",
    ] {
        let source = format!("{class} {MAIN}");
        let program = parse(&source);
        let method = &program.classes[0].methods[0];
        assert_eq!(method.ret, Type::Int);
        assert_eq!(method.name, "add");
        assert_eq!(method.params.len(), 2);
        compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}

#[test]
fn actor_and_static_methods_parse_identical_return_types() {
    let source = format!(
        "define actor Worker as int• count() {{ return 1; }} end \
         define class C as static fnc int• count() {{ return 1; }} end {MAIN}"
    );
    let program = parse(&source);
    assert_eq!(program.actors[0].methods[0].ret, Type::Int);
    assert_eq!(program.classes[0].methods[0].ret, Type::Int);
    assert!(program.classes[0].methods[0].is_static);
}

#[test]
fn nested_generic_builtins_and_callable_type_return_parse() {
    for (ty, expected) in [
        ("Option<int>", Type::OptionInt),
        ("Result<int,string>", Type::ResultIntStr),
        ("Option<Result<int,string>>", Type::OptionResultIntStr),
        ("fnc(int)->int", Type::Callable(0)),
    ] {
        let source = format!("fnc {ty}• factory() {{ return 0; }}");
        let program = parse(&source);
        if matches!(expected, Type::Callable(_)) {
            assert!(matches!(program.funcs[0].ret, Type::Callable(_)));
        } else {
            assert_eq!(program.funcs[0].ret, expected);
        }
    }
}

#[test]
fn identical_postfix_types_warn_but_compile() {
    for header in ["int• f(): int", "fnc int• f() -> int"] {
        let source = format!("{header} {{ return 7; }} {MAIN}");
        let (wasm, warnings) = compile_with_warnings(&source).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "W_DUPLICATE_RETURN_TYPE");
        assert!(warnings[0].pos.col > 0);
    }
    let class = "define class Calculator { int• add(a: int,b: int): int { return a+b; } }";
    let mut parser = Parser::new(lex(class).unwrap());
    parser.parse_program().unwrap();
    assert_eq!(parser.warnings().len(), 1);
    assert_eq!(parser.warnings()[0].code, "W_DUPLICATE_RETURN_TYPE");
}

#[test]
fn duplicate_qualified_named_types_are_compared_by_spelling() {
    let source = "Widget• make(): Widget { return new Widget(); }";
    let mut parser = Parser::new(lex(source).unwrap());
    parser.parse_program().unwrap();
    assert_eq!(parser.warnings()[0].code, "W_DUPLICATE_RETURN_TYPE");
}

#[test]
fn exact_exclusion_of_leading_type_is_rejected_even_with_generics() {
    for source in [
        "int• f(): !int { return 1; }",
        "fnc int• f(): !int { return 1; }",
        "define class Calculator { int• add(a: int,b: int): !int { return a+b; } }",
        "T• identity<T>(value: T): !T { return value; }",
        "define class Calculator { T• identity<T>(value: T): !T { return value; } }",
    ] {
        fails(source, "E_RETURN_EXCLUSION");
    }
}

#[test]
fn conflicting_normal_return_annotation_is_rejected() {
    for source in [
        "int• f(): bool { return 1; }",
        "int• f() -> string { return 1; }",
        "define class C { int• f(): bool { return 1; } }",
    ] {
        fails(source, "E_RETURN_TYPE_CONFLICT");
    }
}

#[test]
fn generic_and_other_exclusion_profiles_fail_closed() {
    for source in [
        "int• f(): !string { return 1; }",
        "T• identity<T>(value: T) { return value; }",
        "fnc T• identity<T>(value: T) { return value; }",
    ] {
        fails(source, "E_UNSUPPORTED");
    }
}

#[test]
fn bullet_token_does_not_escape_strings_or_comments() {
    let tokens = lex("int• f() {} // •\n\"•\"").unwrap();
    assert!(tokens.iter().any(|t| matches!(t.tok, atto_compiler::lexer::Tok::Bullet)));
    assert!(tokens.iter().any(|t| matches!(&t.tok, atto_compiler::lexer::Tok::Str(s) if s == "•")));
}

#[test]
fn malformed_and_bare_type_first_declarations_fail_closed() {
    fails("int foo() { return 1; }", "E_TOPLEVEL");
    for source in [
        "int• f(: int) { return 1; }",
        "int• f() : { return 1; }",
        "int• f(): ! { return 1; }",
        "int•• f() { return 1; }",
        "int• 123() { return 1; }",
        "int• f() { return 1•; }",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn existing_end_style_does_not_scan_into_next_declaration() {
    let source = "fnc f() -> int let int x = 3, x + 2 end int• g() { return 2; } pub fnc main(): int { return f(); }";
    let program = parse(source);
    assert_eq!(program.funcs.len(), 3);
    assert_eq!(program.funcs[0].ret, Type::Int);
    assert_eq!(program.funcs[1].ret, Type::Int);
}

#[test]
fn leading_type_drives_existing_type_checker() {
    fails("int• f() { return true; } pub fnc main(): int { return 0; }", "E_TYPE");
    fails("bool• f() { return 42; } pub fnc main(): int { return 0; }", "E_TYPE");
}

#[test]
fn warnings_do_not_leak_between_compilations() {
    let with = format!("int• f(): int {{ return 1; }} {MAIN}");
    let without = format!("int• f() {{ return 1; }} {MAIN}");
    assert_eq!(compile_with_warnings(&with).unwrap().1.len(), 1);
    assert!(compile_with_warnings(&without).unwrap().1.is_empty());
}

#[test]
fn equivalent_scalar_aliases_warn_and_unsupported_aliases_fail_closed() {
    for header in ["bool• f(): boolean", "float• f(): f64", "string• f(): String"] {
        let source = format!("{header} {{ return 0; }}");
        let mut parser = Parser::new(lex(&source).unwrap());
        parser.parse_program().unwrap();
        assert_eq!(parser.warnings().len(), 1, "{source}");
    }
    // The browser subset does not implement general nominal type aliases.
    fails("int• f(): !string { return 1; }", "E_UNSUPPORTED");
}

#[test]
fn generic_type_parameter_duplicate_is_warned_before_monomorphization_gate() {
    let source = "T• identity<T>(value: T): T { return value; }";
    let mut parser = Parser::new(lex(source).unwrap());
    let err = parser.parse_program().unwrap_err();
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert_eq!(parser.warnings().len(), 1);
    assert_eq!(parser.warnings()[0].code, "W_DUPLICATE_RETURN_TYPE");
}

#[test]
fn comments_and_unicode_positioning_preserve_the_exact_bullet_token() {
    let source = "fnc int /* preceding */ • /* following */ foo() { return 42; }";
    let mut parser = Parser::new(lex(source).unwrap());
    let program = parser.parse_program().unwrap();
    assert_eq!(program.funcs[0].ret, Type::Int);
    let bullet = lex("int• foo() {}").unwrap().into_iter()
        .find(|token| matches!(token.tok, atto_compiler::lexer::Tok::Bullet))
        .unwrap();
    assert_eq!(bullet.pos.line, 1);
    assert_eq!(bullet.pos.col, 4);
}
