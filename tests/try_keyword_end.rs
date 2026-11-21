//! End-delimited imperative exception blocks must reuse the existing EH AST.
use atto_compiler::compile;

fn accepts(source: &str) {
    let bytes = compile(source).unwrap_or_else(|e| panic!("{e}\n{source}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(source).unwrap(), "deterministic native compiler");
}

fn rejects(source: &str) {
    let e = compile(source).expect_err("malformed try must fail before emission");
    assert!(e.pos.line > 0 && e.pos.col > 0, "{e}");
}

#[test]
fn keyword_and_braced_finally_compile_to_wasm() {
    for block in [
        "try { return 7; } finally { stdio.println(1); }",
        "try\n return 7;\nfinally\n stdio.println(1);\nend",
        "try\n try { return 7; } finally { stdio.println(1); }\nfinally\n stdio.println(2);\nend",
        "try\n try\n return 7;\nfinally\n stdio.println(1);\nend\nfinally\n stdio.println(2);\nend"
    ] {
        accepts(&format!("pub fnc main(): int {{\n{block}\n}}"));
    }
}

#[test]
fn catch_empty_and_typed_catch_compile() {
    accepts(r#"
        fnc fail(): int { throw Error("bad"); }
        pub fnc main(): int {
          try
            return fail();
          catch()
            return 9;
          end
        }
    "#);
    accepts(r#"
        fnc fail(): int { throw Error("bad"); }
        pub fnc main(): int {
          try
            return fail();
          catch()
            return 9;
          finally
            stdio.println("cleanup");
          end
        }
    "#);
    accepts(r#"
        fnc fail(): int { throw Error("bad"); }
        pub fnc main(): int {
          try { return fail(); } catch (Error e) { return 9; }
        }
    "#);
    accepts(r#"
        fnc fail(): int { throw Error("bad"); }
        pub fnc main(): int {
          try { return fail(); } catch() { return 9; }
        }
    "#);
}

#[test]
fn missing_end_duplicate_handlers_and_mixed_styles_are_rejected() {
    for block in [
        "try\n return 1;\nend",
        "try\n return 1;\nfinally\n stdio.println(1);",
        "try\n return 1;\ncatch()\n return 2;\ncatch()\n return 3;\nend",
        "try\n return 1;\nfinally\n stdio.println(1);\ncatch()\n return 2;\nend",
        "try\n return 1;\ncatch() { return 2; }\nend",
        "try { return 1; } catch()\n return 2;\nend",
    ] {
        rejects(&format!("pub fnc main(): int {{\n{block}\n}}"));
    }
}

#[test]
fn maybe_value_matching_nested_in_keyword_try_must_fail_closed() {
    // Parsing try/catch does not admit the still-unimplemented maybe
    // value/cleanup CFG. Neither a match failure nor a thrown exception
    // may be reinterpreted through the other's control-flow mechanism.
    for body in [
        "do maybe\n Ok(value) ?= Ok(5);\n return value;\nelse\n Err(e) -> { return 0; };\nend",
        "maybe\n Ok(value) ?= Ok(5);\n value\nelse\n on Err(e) -> 0\nend",
        "return maybe\n Ok(value) ?= Ok(5);\n value\nelse\n Err(e) -> { return 0; };\nend;",
    ] {
        let source = format!(
            "pub fnc main(): int {{\ntry\n{body}\ncatch()\n return 0;\nend\n}}"
        );
        let failure = compile(&source).expect_err("unimplemented maybe must not emit Wasm");
        assert_eq!(failure.code, "E_UNSUPPORTED", "{failure}: {body}");
    }
}

#[test]
fn contextual_maybe_identifier_is_legal_but_fallible_scope_stays_unsupported() {
    accepts(r#"
        fnc maybe(): int { return 7; }
        pub fnc main(): int { return maybe(); }
    "#);
    accepts(r#"
        pub fnc main(): int {
            val maybe: int = 7;
            return maybe;
        }
    "#);
    let unsupported = r#"
        pub fnc main(): int {
          try
            return maybe
              Ok(value) ?= Ok(7);
              value
            else
              Err(reason) -> { return 0; };
            end;
          finally
            return 9;
          end
        }
    "#;
    let failure = compile(unsupported).expect_err("fallible value scope requires typed CFG");
    assert_eq!(failure.code, "E_UNSUPPORTED", "{failure}");
}
