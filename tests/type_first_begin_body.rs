//! Explicit begin/end around a type-first callable body.
//! These tests exercise production parsing and WebAssembly lowering, not a
//! syntax-only lookahead scanner.
use atto_compiler::{ast::Type, compile, lexer::lex, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lex(source).unwrap()).parse_program()
        .unwrap_or_else(|error| panic!("{source}: {error}"))
}

fn with_main(header: &str, body: &str, close: &str) -> String {
    format!("{header}\nbegin\n{body}\n{close}\npub fnc main(): int {{ return foo(); }}")
}

#[test]
fn type_first_begin_end_accepts_four_headers_and_three_closers() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        for close in ["end", "end.", "end;"] {
            let src = with_main(header, "return 42;", close);
            let program = parse(&src);
            assert_eq!(program.funcs.len(), 2, "{src}");
            assert_eq!(program.funcs[0].ret, Type::Int, "{src}");
            let wasm = compile(&src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "{src}");
            assert_eq!(wasm, compile(&src).unwrap(), "{src}");
        }
    }
}

#[test]
fn begin_end_is_indentation_independent_and_accepts_statement_newlines() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        for body in ["return 42;", "  return 42;", "\treturn 42", "return 42"] {
            let src = with_main(header, body, "end.");
            assert!(compile(&src).is_ok(), "{src}");
        }
    }
}

#[test]
fn braced_and_bare_end_bodies_still_compile_without_begin() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        for src in [
            format!("{header} {{ return 42; }}\npub fnc main(): int {{ return foo(); }}"),
            format!("{header}\nreturn 42;\nend.\npub fnc main(): int {{ return foo(); }}"),
        ] {
            assert!(compile(&src).is_ok(), "{src}");
        }
    }
}

#[test]
fn begin_inside_class_actor_and_static_method_headers_nests_correctly() {
    for delim in ["<-", "•"] {
        let src = format!(
            "define class C as\nfnc int {delim} first()\nbegin\nreturn 1;\nend.\n             mtd int {delim} second()\nbegin\nreturn 2;\nend;\n             static fnc int {delim} third()\nbegin\nreturn 3;\nend\nend\n             define actor A as\nmtd int {delim} count()\nbegin\nreturn 4;\nend.\nend\n             pub fnc main(): int {{ return 0; }}"
        );
        let program = parse(&src);
        assert_eq!(program.classes.len(), 1, "{src}");
        assert_eq!(program.classes[0].methods.len(), 3, "{src}");
        assert!(program.classes[0].methods[2].is_static);
        assert_eq!(program.actors[0].methods.len(), 1, "{src}");
        // The actor parser is currently parse-only for selected methods.
        assert_eq!(program.actors[0].methods[0].ret, Type::Int);
    }
}

#[test]
fn nested_keyword_if_and_return_match_do_not_steal_callable_end() {
    for delim in ["<-", "•"] {
        let src = format!(
            "int {delim} foo(n: int)\nbegin\n             if n > 0 then\nreturn 42;\nelse\nreturn match n over\non 0 -> 7,\ndefault -> 0,\nend\nfi\nend.\n             pub fnc main(): int {{ return foo(0); }}"
        );
        let parsed = parse(&src);
        assert_eq!(parsed.funcs.len(), 2, "{src}");
    }
}

#[test]
fn invalid_begin_end_structures_reject_without_synthesizing_code() {
    for header in ["int <- foo()", "int• foo()", "fnc int <- foo()", "fnc int• foo()"] {
        for body in [
            format!("{header}\nbegin\nreturn 42;"),
            format!("{header}\nbegin\nend."),
            format!("{header}\nbegin\nreturn 42;\nend.."),
            format!("{header}\nbegin\nreturn 42;\nend;;"),
            format!("{header}\nbegin\nreturn 42;\nend;." ),
            format!("{header}\nbegin\nreturn 42,\nend."),
            format!("{header}\nbegin\nreturn true;\nend."),
        ] {
            assert!(compile(&body).is_err(), "{body}");
        }
    }
}

#[test]
fn begin_is_not_a_top_level_function_or_an_alias_for_unknown_bullet() {
    for src in [
        "mtd int• foo()\nbegin\nreturn 42;\nend.",
        "int● foo()\nbegin\nreturn 42;\nend.",
        "int < - foo()\nbegin\nreturn 42;\nend.",
    ] {
        assert!(compile(src).is_err(), "{src}");
    }
}

#[test]
fn top_level_declarations_follow_closed_begin_blocks() {
    for delim in ["<-", "•"] {
        let src = format!(
            "int {delim} foo()\nbegin\nreturn 42;\nend.\n             int {delim} bar()\nbegin\nreturn foo();\nend;\n             pub fnc main(): int {{ return bar(); }}"
        );
        let program = parse(&src);
        assert_eq!(program.funcs.len(), 3, "{src}");
        assert!(compile(&src).is_ok(), "{src}");
    }
}
