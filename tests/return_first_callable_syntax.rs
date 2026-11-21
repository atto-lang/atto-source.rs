//! Return-first <- and begin/end syntax. These forms are alternative
//! declarations; duplicate names in one unit remain an error. Runtime
//! conformance requires actually executing the compiled guest.
use atto_compiler::{compile, lexer::{lex, Tok}, parser::Parser};
use atto_compiler::ast::Type;
use atto_compiler::target_syntax::parse_target_declarations;

const INT_FORMS: &[&str] = &[
    "fnc calculate(int x) -> int { return x * 2; }",
    "fnc calculate(int x): int { return x * 2; }",
    "fnc calculate(int x): int -> { return x * 2; }",
    "fnc int <- calculate(int x) { return x * 2; }",
    "int <- calculate(int x) { return x * 2; }",
    "int <- calculate(int x) return x * 2; end",
    "int <- calculate(int x) begin return x * 2; end",
    "fnc int <- calculate(int x) begin return x * 2; end",
    "fnc int <- calculate(int x) ::\n  return x * 2;\nend",
    "int <- calculate(int x) ::\n  return x * 2;\nend",
    "fnc int• calculate(int x) ::\n  return x * 2;\nend",
    "int• calculate(int x) { return x * 2; }",
    "fnc int• calculate(int x) begin return x * 2; end",
    "int• calculate(int x) return x * 2; end",
    "fnc calculate(int x) -> int begin return x * 2; end",
];

#[test]
fn bullet_is_exact_utf8_return_delimiter_not_any_similar_glyph() {
    let tokens = lex("int• answer()").expect("lex exact U+2022 bullet");
    assert_eq!(tokens[1].tok, Tok::LeftArrow);
    assert_eq!((tokens[1].pos.line, tokens[1].pos.col), (1, 4));
    for glyph in ["·", "∙", "‿", "‧", "●"] {
        let source = format!("int{glyph} answer() begin return 42; end");
        let err = lex(&source).expect_err("lookalike bullet must not be normalized");
        assert_eq!(err.code, "E_TOKEN", "{source}");
        assert_eq!((err.pos.line, err.pos.col), (1, 4), "{source}");
    }
}

#[test]
fn all_requested_int_forms_share_signature_and_compile_to_wasm() {
    for header in INT_FORMS {
        let source = format!("{header}\npub fnc main() -> int {{ return calculate(21); }}");
        let p = Parser::new(lex(&source).unwrap()).parse_program().unwrap();
        assert_eq!(p.funcs[0].name, "calculate", "{header}");
        assert_eq!(p.funcs[0].params.len(), 1, "{header}");
        assert_eq!(p.funcs[0].ret, Type::Int, "{header}");
        let guest = compile(&source).unwrap();
        assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0], "{header}");
    }
}

#[test]
fn void_forms_support_empty_brace_begin_and_bare_end_bodies() {
    for header in [
        "fnc reset() -> void { }",
        "fnc reset(): void { }",
        "fnc reset(): void -> { }",
        "fnc void <- reset() { }",
        "void <- reset() { }",
        "void <- reset() end",
        "void <- reset() begin end",
        "fnc void <- reset() begin end",
    ] {
        let src = format!("{header}\npub fnc main() -> int {{ reset(); return 0; }}");
        let p = Parser::new(lex(&src).unwrap()).parse_program().unwrap();
        assert_eq!(p.funcs[0].ret, Type::Void, "{header}");
        assert!(!compile(&src).unwrap().is_empty(), "{header}");
    }
}

#[test]
fn infer_forms_resolve_int_and_do_not_fallback_to_void() {
    for header in [
        "fnc calculate_other() -> infer { return 42; }",
        "fnc calculate_other(): infer { return 42; }",
        "fnc calculate_other(): infer -> { return 42; }",
        "fnc infer <- calculate_other() { return 42; }",
        "infer <- calculate_other() { return 42; }",
        "infer <- calculate_other() return 42; end",
        "infer <- calculate_other() begin return 42; end",
        "fnc infer <- calculate_other() begin return 42; end",
    ] {
        let src = format!("{header}\npub fnc main() -> int {{ return calculate_other(); }}");
        let p = Parser::new(lex(&src).unwrap()).parse_program().unwrap();
        assert_eq!(p.funcs[0].ret, Type::Int, "{header}");
        assert!(!compile(&src).unwrap().is_empty(), "{header}");
    }
}

#[test]
fn return_first_inline_methods_obey_normal_class_method_semantics() {
    let src = r#"
        define class Counter as
          int <- calculate(int x) { return x * 2; }
          fnc void <- reset() begin end
          infer <- double(int x) begin return x * 2; end
        end
        pub fnc main() -> int { return 0; }
    "#;
    let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(p.classes[0].methods.len(), 3);
    assert_eq!(p.classes[0].methods[0].ret, Type::Int);
    assert_eq!(p.classes[0].methods[1].ret, Type::Void);
    assert_eq!(p.classes[0].methods[2].ret, Type::Int);
}

#[test]
fn return_first_target_header_scanner_preserves_signature() {
    let src = "fnc int <- calculate(int x) begin return x * 2; end";
    let headers = parse_target_declarations(&lex(src).unwrap()).unwrap();
    assert!(headers.receivers.is_empty());
    let src2 = "int <- calculate(int x) return x * 2; end";
    parse_target_declarations(&lex(src2).unwrap()).unwrap();
}

#[test]
fn invalid_return_first_forms_reject_before_guest_emission() {
    for (decl, code) in [
        ("int < - calculate(int x) { return x; }", "E_TOPLEVEL"),
        ("int <- calculate(int x) -> int { return x; }", "E_RETURN_ANNOTATION"),
    ] {
        let src = format!("{decl}\npub fnc main() -> int {{ return 0; }}");
        let d = compile(&src).unwrap_err();
        assert_eq!(d.code, code, "{decl}: {d}");
    }
}

#[test]
fn return_first_unclosed_begin_requires_matching_end() {
    let d = compile("int <- calculate(int x) begin return x;").unwrap_err();
    assert_eq!(d.code, "E_EOF", "{d}");
}

#[test]
fn return_first_nonvoid_requires_a_return_on_every_path() {
    for decl in [
        "int <- calculate(int x) begin end",
        "int <- calculate(int x) end",
    ] {
        let src = format!("{decl}\npub fnc main() -> int {{ return 0; }}");
        let d = compile(&src).unwrap_err();
        assert_eq!(d.code, "E_RETURN", "{decl}: {d}");
    }
}

#[test]
fn duplicate_spelling_variants_do_not_create_overloads() {
    let src = r#"
        fnc calculate(int x) -> int { return x; }
        int <- calculate(int x) begin return x; end
        pub fnc main() -> int { return 0; }
    "#;
    let d = compile(src).unwrap_err();
    assert_eq!(d.code, "E_DUPLICATE", "{d}");
}

#[test]
fn left_arrow_is_one_contiguous_header_token() {
    let packed = lex("int<-calculate(int x) { return x; }").unwrap();
    assert!(packed.iter().any(|t| t.tok == Tok::LeftArrow));
    let spaced = lex("int < - calculate(int x) { return x; }").unwrap();
    assert!(!spaced.iter().any(|t| t.tok == Tok::LeftArrow));
    assert!(spaced.iter().any(|t| t.tok == Tok::Lt));
    let relational = lex("int <= x").unwrap();
    assert!(relational.iter().any(|t| t.tok == Tok::Le));
}

#[test]
fn explicit_begin_end_preserves_void_and_inferred_void() {
    for declaration in [
        "fnc empty(): void begin end",
        "fnc inferred() -> infer begin end",
        "void <- empty() begin end",
        "infer <- inferred() begin end",
    ] {
        let src = format!("{declaration}\npub fnc main() -> int {{ return 0; }}");
        let p = Parser::new(lex(&src).unwrap()).parse_program().unwrap();
        assert_eq!(p.funcs[0].ret, Type::Void, "{declaration}");
    }
}

#[test]
fn nested_generic_return_first_headers_preserve_complete_declared_type() {
    // The parser must scan over balanced '<...>' before locating '<-';
    // return-first is not restricted to a single-token scalar type.
    let src = r#"
        fnc Option<int> <- optional() { return Some(7); }
        Result<int,string> <- result() begin return Ok(8); end
        Option<Result<int,string>> <- nested() return None; end
        define class C as
          Option<int> <- value() { return Some(9); }
        end
        pub fnc main() -> int { return 0; }
    "#;
    let p = Parser::new(lex(src).unwrap()).parse_program().unwrap();
    assert_eq!(p.funcs[0].ret, Type::OptionInt);
    assert_eq!(p.funcs[1].ret, Type::ResultIntStr);
    assert_eq!(p.funcs[2].ret, Type::OptionResultIntStr);
    assert_eq!(p.classes[0].methods[0].ret, Type::OptionInt);
}

#[test]
fn return_first_out_of_line_class_method_compiles_through_checked_guest_lowering() {
    // Out-of-line mtd must use the same typed class method checker, not a
    // syntax-only shortcut or an unchecked receiver dispatch path.
    for body in [
        "{ return self.value; }",
        "begin return self.value; end",
        "return self.value; end",
    ] {
        let src = format!(r#"
            declare class Counter;
            define class Counter as
              pub let int value = 0;
            end
            pub constructor (Counter)(int initial): self {{
              self.value = initial;
            }}
            pub mtd (Counter self) int <- get_value() {body}
            pub fnc main() -> int {{
              val counter = new Counter(42);
              return counter.get_value();
            }}
        "#);
        let guest = compile(&src).unwrap_or_else(|e|
            panic!("return-first out-of-line class method {body}: {e}"));
        assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
}

#[test]
fn return_first_colon_colon_is_explicit_statement_body_and_needs_a_newline() {
    for decl in [
        "fnc int <- answer() ::\n  return 42;\nend",
        "int• answer() ::\n  return 42;\nend",
        "fnc int • answer() ::\n  return 42;\nend",
    ] {
        let src = format!("{decl}\npub fnc main(): int {{ return answer(); }}");
        let wasm = compile(&src).unwrap_or_else(|err| panic!("{decl}: {err}"));
        assert_eq!(&wasm[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
    for (decl, expected) in [
        ("int <- answer() :: return 42; end", "E_SYNTAX"),
        ("int <- answer() ::\n :: return 42; end", "E_SYNTAX"),
        ("int <- answer() ::\n { return 42; }", "E_SYNTAX"),
        ("int <- answer() ::\n  42; end", "E_RETURN"),
    ] {
        let src = format!("{decl}\npub fnc main(): int {{ return 0; }}");
        let err = compile(&src).expect_err("invalid return-first :: body");
        assert_eq!(err.code, expected, "{decl}: {err}");
    }
    let missing_end = compile("int <- answer() ::\n  return 42;")
        .expect_err("end delimiter is required");
    assert_eq!(missing_end.code, "E_EOF", "{missing_end}");
}


#[test]
fn return_first_checked_error_effects_support_free_form_clause_and_colon_colon() {
    for decl in [
        "fnc int <- twice(int value)\nraises Error ::\n  return value * 2;\nend",
        "fnc int • twice(int value)\nraises Error\nbegin\n  return value * 2;\nend",
        "fnc int <- twice(int value)\nthrows Error ::\n  return value * 2;\nend",
        "fnc int <- twice(int value)\nraises(Error) ::\n  return value * 2;\nend",
    ] {
        let src = format!("{decl}\npub fnc main(): int {{ return twice(21); }}");
        let wasm = compile(&src).unwrap_or_else(|err| panic!("{decl}: {err}"));
        assert_eq!(&wasm[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
}

#[test]
fn return_first_checked_raise_bang_is_not_an_ordinary_throw() {
    let src = r#"
        fnc int <- twice(int value)
        raises Error ::
          if value > 3 then
            raise! Error("too large");
          fi
          return value * 2;
        end
        pub fnc main(): int raises(Error) {
          return twice(3) + 36;
        }
    "#;
    let wasm = compile(src).unwrap();
    assert_eq!(&wasm[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    let contiguous = lex(r#"raise! Error("bad");"#).unwrap();
    assert!(matches!(&contiguous[0].tok, Tok::Ident(name) if name == "raise!"));
    let separate = lex(r#"raise !Error("bad");"#).unwrap();
    assert!(matches!(&separate[0].tok, Tok::Ident(name) if name == "raise"));
    assert_eq!(separate[1].tok, Tok::Bang);
}

#[test]
fn return_first_effect_boundary_and_canonical_exception_bangs_fail_closed() {
    for (decl, code) in [
        ("fnc int <- twice(int x)\nraises Foo ::\n  return x * 2;\nend", "E_TYPE"),
        ("fnc int <- twice(int x)\nraises Error\n  return x * 2;\nend", "E_SYNTAX"),
        ("fnc int <- twice(int x)\nraises Error raises Error ::\n  return x * 2;\nend", "E_DUPLICATE"),
        ("fnc int <- twice(int x)\nraises Error :: return x * 2; end", "E_SYNTAX"),
        ("fnc int <- twice(int x)\nraises Error ::\n  42;\nend", "E_RETURN"),
        ("fnc int <- twice(int x)\nraises Error ::\n  panic! Error(\"no control\");\nend", "E_UNSUPPORTED"),
    ] {
        let src = format!("{decl}\npub fnc main(): int {{ return 0; }}");
        let err = compile(&src).expect_err("invalid source must emit no guest");
        assert_eq!(err.code, code, "{decl}: {err}");
    }
    let bad = compile("pub fnc main(): int { val x = trap! 42; return 42; }")
        .expect_err("legacy Option trap is not canonical Result");
    assert_eq!(bad.code, "E_TRAP_BANG_RESULT_UNVERIFIED");
}


#[test]
fn triggered_return_first_raise_has_recover_catch_and_trap_guest_targets() {
    for fixture in [
        include_str!("fixtures/pass/return-first-raise-recover-guest.atto"),
        include_str!("fixtures/pass/return-first-raise-bypasses-catch.atto"),
        include_str!("fixtures/pass/return-first-raise-bypasses-trap.atto"),
    ] {
        assert!(fixture.contains("raise! Error("));
        let guest = compile(fixture).unwrap_or_else(|err|
            panic!("triggered-raise candidate compiler rejection: {err}"));
        assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
}

#[test]
fn checked_exception_effect_mixes_and_missing_raises_fail_closed() {
    // An effect comma separates kinds, not additional unverified error types.
    for header in [
        "throws(Error), raises(Error)",
        "throws Error, raises Error",
        "throws Error raises(Error)",
        "raises(Error), throws Error",
    ] {
        let src = format!("fnc int <- risky(int x)\n{header} ::\n  return x;\nend\npub fnc main(): int {{ return risky(42); }}");
        let wasm = compile(&src).unwrap_or_else(|err| panic!("{header}: {err}"));
        assert_eq!(&wasm[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
    for (header, code) in [
        ("raises Error, Foo", "E_TYPE"),
        ("raises(Error, Error)", "E_DUPLICATE"),
        ("raises Error, raises Error", "E_DUPLICATE"),
        ("raises Error,", "E_TYPE"),
        ("raises Foo", "E_TYPE"),
    ] {
        let src = format!("fnc int <- risky(int x)\n{header} ::\n  return x;\nend\npub fnc main(): int {{ return 0; }}");
        let err = compile(&src).expect_err("invalid effect must produce no Wasm");
        assert_eq!(err.code, code, "{header}: {err}");
    }
    let undeclared = concat!(
        "fnc int <- risky(int x)\nthrows Error ::\n",
        "  raise! Error(\"unaccounted\");\nend\n",
        "pub fnc main(): int { return 0; }\n"
    );
    let err = compile(undeclared).expect_err("raises must not pass throws-only bound");
    assert_eq!(err.code, "E_EFFECT", "{err}");
}


#[test]
fn explicit_error_inheritance_preserves_nominal_checked_effect_identity() {
    // Both declarations orders are legal; the parser must retain the exact
    // nominal type and the checker must validate the Error ancestry.
    for body in [
        concat!(
            "define class Foo extends Error as end\n",
            "fnc int • twice(int value)\nraises Foo ::\n",
            "  return value * 2;\nend\n",
        ),
        concat!(
            "fnc int <- twice(int value)\nraises(Foo)\nbegin\n",
            "  return value * 2;\nend\n",
            "define class Foo extends Error as end\n",
        ),
    ] {
        let source = format!("{body}pub fnc main(): int {{ return twice(21); }}");
        let p = Parser::new(lex(&source).unwrap()).parse_program().unwrap();
        assert_eq!(p.classes.len(), 1, "{body}");
        assert_eq!(p.classes[0].name, "Foo");
        assert!(p.classes[0].error_parent, "no implicit or missing ancestry");
        let nominal = p.funcs[0].declared_effects.unwrap().raises_nominal
            .expect("Foo effect must carry a nominal identity");
        assert_eq!(p.type_names[nominal as usize].0, "Foo");
        let guest = compile(&source).unwrap_or_else(|err|
            panic!("validated, non-raising Foo contract rejected: {err}"));
        assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    }
}

#[test]
fn nominal_error_effects_reject_non_error_or_unknown_types() {
    for (prefix, annotation, code) in [
        ("define class Bar as end\n", "raises Bar", "E_TYPE"),
        ("", "raises Missing", "E_TYPE"),
        ("define class Foo extends Object as end\n", "raises Foo", "E_UNSUPPORTED"),
        ("define class Foo as end\n", "throws Foo", "E_TYPE"),
    ] {
        let source = format!(
            "{prefix}fnc int <- risky(int x)\n{annotation} ::\n  return x;\nend\n\
             pub fnc main(): int {{ return risky(42); }}"
        );
        let err = compile(&source).expect_err("unproved Error subtype must emit no guest");
        assert_eq!(err.code, code, "{annotation}: {err}");
    }
}

#[test]
fn nominal_error_runtime_payload_must_not_silently_degrade_to_string_tag() {
    let raised = r#"
        define class Foo extends Error as end
        fnc int <- risky(int value)
        raises Foo ::
          if value > 3 then
            raise! new Foo();
          fi
          return value * 2;
        end
        pub fnc main(): int { return risky(3); }
    "#;
    let err = compile(raised).expect_err(
        "unverified subtype-bearing Wasm EH must not produce a guest");
    assert_eq!(err.code, "E_UNSUPPORTED", "{err}");

    let incorrectly_narrowed = r#"
        define class Foo extends Error as end
        fnc int <- risky(int value)
        raises Foo ::
          raise! Error("not a Foo");
        end
        pub fnc main(): int { return 0; }
    "#;
    let err = compile(incorrectly_narrowed).expect_err(
        "declaring Foo must not silently authorize a generic Error");
    assert_eq!(err.code, "E_EFFECT", "{err}");
}
