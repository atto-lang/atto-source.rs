//! Integrated source failure rows with executable Result traps and unwinding.
//! Nominal Error subtype generalization and panic-freedom remain gated.
use atto_compiler::{compile, lexer, parser::Parser};

fn accepted(src: &str) {
    let compiled = compile(src).unwrap_or_else(|e| panic!("unexpected {e}\n{src}"));
    assert_eq!(&compiled[..8], b"\0asm\x01\0\0\0");
}
fn rejected(src: &str, expected: &str) {
    let error = compile(src).expect_err("invalid effects must emit no Wasm");
    assert_eq!(error.code, expected, "{error}\n{src}");
}
#[test]
fn retains_explicit_empty_bounds_independently_from_inference() {
    let src = "@Throws() @Raises(Error)\nfnc test(): int { return 2; }\npub fnc main(): int { return test(); }";
    let ast = Parser::new(lexer::lex(src).expect("lex")).parse_program().expect("parse");
    let effects = &ast.funcs[0].failure_effects;
    assert!(effects.explicit);
    assert_eq!(effects.declared, [true,true,false]);
    assert!(effects.throws.is_empty());
    assert_eq!(effects.raises, vec!["Error".to_owned()]);
    accepted(src);
}
#[test]
fn trapped_ordinary_throw_discharges_explicit_empty_bounds() {
    accepted(r#"
        fnc fail(): int throws(Error) { throw Error("ordinary"); }
        @Throws() @Raises()
        pub fnc main(): int {
            val Result<int,Error> result = trap fail();
            match result over
              on Ok(n) -> { return n; }
              on Err(err) -> { return 42; }
            end
        }
    "#);
}
#[test]
fn partial_rows_bound_transitive_escaping_throw_and_raise() {
    rejected(r#"
        fnc fail(): int { throw Error("escaped"); }
        @Raises(Error)
        fnc exposed(): int { return fail(); }
        pub fnc main(): int { return 1; }
    "#, "E_EFFECT");
    rejected(r#"
        @Raises()
        fnc fail(): int { raise Error("escaped"); }
        pub fnc main(): int { return 1; }
    "#, "E_EFFECT");
    accepted(r#"
        fnc fail(): int { throw Error("caught"); }
        @Throws()
        fnc safe(): int {
            try { return fail(); }
            catch (Error e) { return 7; }
        }
        pub fnc main(): int { return safe(); }
    "#);
}
#[test]
fn annotation_and_java_style_clause_share_one_typed_row() {
    accepted(r#"
        @Throws(Error)
        fnc risky(): int { throw Error("x"); }
        pub fnc main(): int { return 3; }
    "#);
    accepted(r#"
        fnc risky(): int throws Error { throw Error("x"); }
        pub fnc main(): int { return 3; }
    "#);
    rejected(r#"
        @Throws(Error)
        fnc f(): int throws(Error) { return 1; }
        pub fnc main(): int { return 0; }
    "#, "E_EFFECT_TYPE");
    rejected(r#"
        @Throws(UnprovedError)
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED_TYPE");
    rejected(r#"
        @Throws()
        @Throws(Error)
        pub fnc main(): int { return 0; }
    "#, "E_EFFECT_TYPE");
}
#[test]
fn malformed_annotations_fail_closed() {
    rejected("@Throws pub fnc main(): int { return 0; }", "E_SYNTAX");
    rejected("@Custom() pub fnc main(): int { return 0; }", "E_UNSUPPORTED");
    rejected("@Throws() define class Foo as end\npub fnc main(): int {return 0;}", "E_SYNTAX");
}
