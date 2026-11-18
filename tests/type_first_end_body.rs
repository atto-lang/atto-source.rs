//! Executable regression matrix for type-first newline/end callable bodies.
//! The U+25CF BLACK CIRCLE is intentionally not a return delimiter.
use atto_compiler::{ast::Type, compile, compile_with_warnings, lexer::lex, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(source).unwrap()).parse_program()
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn fails(source: &str, code: &str) {
    let err = compile(source).expect_err(source);
    assert_eq!(err.code, code, "{source}: {err}");
}

fn source(header: &str, stmt: &str, finish: &str) -> String {
    format!("{header}\n{stmt}\n{finish}\npub fnc main(): int {{ return foo(); }}")
}

#[test]
fn four_prefix_spellings_and_three_end_terminators_compile() {
    for header in [
        "int <- foo()",
        "int• foo()",
        "fnc int <- foo()",
        "fnc int• foo()",
    ] {
        for finish in ["end", "end.", "end;"] {
            let src = source(header, "    return 42;", finish);
            let prog = parse(&src);
            assert_eq!(prog.funcs.len(), 2, "{src}");
            assert_eq!(prog.funcs[0].name, "foo", "{src}");
            assert_eq!(prog.funcs[0].ret, Type::Int, "{src}");
            let wasm = compile(&src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{src}");
            assert_eq!(wasm, compile(&src).unwrap(), "{src}");
        }
    }
}

#[test]
fn physical_newline_can_follow_comments_and_body_need_not_be_indented() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        for comment in ["", " // comment", " /* comment */"] {
            for stmt in ["return 42;", "return 42"] {
                let src = source(&format!("{header}{comment}"), stmt, "end.");
                let wasm = compile(&src).unwrap_or_else(|e| panic!("{src}: {e}"));
                assert!(wasm.starts_with(b"\0asm"), "{src}");
            }
        }
    }
}

#[test]
fn braces_still_work_on_one_line_and_without_leading_newline() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        let src = format!("{header} {{ return 42; }}\npub fnc main(): int {{ return foo(); }}");
        assert!(compile(&src).is_ok(), "{src}");
    }
}

#[test]
fn adjacent_declarations_and_nested_class_ends_are_not_swallowed() {
    for delim in ["<-", "•"] {
        let methods = format!(
            "define class C as\nint {delim} first()\nreturn 1;\nend.\n\
             fnc int {delim} second()\nreturn 2;\nend;\nend\n\
             define actor A as\nint {delim} receive_number()\nreturn 7;\nend.\nend\n\
             pub fnc main(): int {{ return 0; }}"
        );
        let program = parse(&methods);
        assert_eq!(program.classes.len(), 1, "{methods}");
        assert_eq!(program.classes[0].methods.len(), 2, "{methods}");
        assert_eq!(program.classes[0].methods[0].ret, Type::Int);
        assert_eq!(program.classes[0].methods[1].ret, Type::Int);
        assert_eq!(program.actors.len(), 1);
        assert_eq!(program.actors[0].methods[0].ret, Type::Int);
    }
}

#[test]
fn static_methods_and_braced_class_syntax_mix_with_end_bodies() {
    for delim in ["<-", "•"] {
        let src = format!(
            "define class C {{\nstatic fnc int {delim} answer()\nreturn 42;\nend;\n}}\n\
             pub fnc main(): int {{ return 0; }}"
        );
        let program = parse(&src);
        assert_eq!(program.classes[0].methods[0].ret, Type::Int);
        assert!(program.classes[0].methods[0].is_static);
    }
}

#[test]
fn nested_if_fi_does_not_close_the_outer_callable() {
    for header in ["int <- foo(n: int)", "int• foo(n: int)"] {
        let src = format!(
            "{header}\nif n > 0 then\nreturn 42;\nelse\nreturn 0;\nfi\nend.\n\
             pub fnc main(): int {{ return foo(3); }}"
        );
        let prog = parse(&src);
        assert_eq!(prog.funcs.len(), 2, "{src}");
        assert!(compile(&src).is_ok(), "{src}");
    }
}

#[test]
fn nested_match_end_does_not_close_the_outer_callable() {
    for header in ["int <- foo(n: int)", "int• foo(n: int)"] {
        let src = format!(
            "{header}\nreturn match n over\non 0 -> 7,\ndefault -> 42,\nend\nend.\n\
             pub fnc main(): int {{ return foo(0); }}"
        );
        let prog = parse(&src);
        assert_eq!(prog.funcs.len(), 2, "{src}");
        assert_eq!(prog.funcs[0].ret, Type::Int);
    }
}

#[test]
fn duplicate_and_conflicting_postfix_types_keep_original_diagnostics() {
    for delim in ["<-", "•"] {
        let same = source(&format!("int {delim} foo(): int"), "return 42;", "end;");
        let (_, warnings) = compile_with_warnings(&same).unwrap();
        assert_eq!(warnings.len(), 1, "{same}");
        assert_eq!(warnings[0].code, "W_DUPLICATE_RETURN_TYPE", "{same}");
        let clean = source(&format!("int {delim} foo()"), "return 42;", "end.");
        assert!(compile_with_warnings(&clean).unwrap().1.is_empty());
        fails(&source(&format!("int {delim} foo(): bool"), "return 42;", "end."),
            "E_RETURN_TYPE_CONFLICT");
        fails(&source(&format!("int {delim} foo(): !int"), "return 42;", "end."),
            "E_RETURN_EXCLUSION");
    }
}

#[test]
fn explicit_return_type_is_checked_for_newline_bodies() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        fails(&source(header, "return true;", "end."), "E_TYPE");
    }
}

#[test]
fn same_line_unbraced_body_and_unterminated_body_are_rejected() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        fails(&format!("{header} return 42;\nend."), "E_SYNTAX");
        assert!(compile(&format!("{header}\nreturn 42;")).is_err());
        assert!(compile(&format!("{header}\nend.")).is_err());
        assert!(compile(&format!("{header}\nreturn 42;\nend..")).is_err());
        assert!(compile(&format!("{header}\nreturn 42;\nend;;")).is_err());
    }
}

#[test]
fn punctuation_without_end_and_wrong_delimiters_fail_closed() {
    for bad in [
        "int• foo()\nreturn 42;\n.",
        "int <- foo()\nreturn 42;\n;",
        "int < - foo()\nreturn 42;\nend.",
        "int● foo()\nreturn 42;\nend.",
        "int•• foo()\nreturn 42;\nend.",
    ] {
        assert!(compile(bad).is_err(), "{bad}");
    }
}

#[test]
fn unrelated_less_than_unary_minus_expression_stays_intact() {
    let src = "bool• foo()\nreturn 1<-2;\nend.\npub fnc main(): int { return 0; }";
    assert!(compile(src).is_ok(), "{src}");
}

#[test]
fn old_trailing_arrow_end_body_and_new_prefix_can_coexist() {
    let src = "fnc old() -> int\nreturn 2;\nend\nint• foo()\nreturn 42;\nend.\n\
               pub fnc main(): int { return old() + foo(); }";
    let prog = parse(src);
    assert_eq!(prog.funcs.len(), 3);
    assert_eq!(prog.funcs[0].ret, Type::Int);
    assert_eq!(prog.funcs[1].ret, Type::Int);
}
