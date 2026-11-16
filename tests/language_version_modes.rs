//! Pre-release v1/v2 callable-body compatibility regression matrix.
//! Defaults are not a stable edition/manifest promise; callers opt in here.
use atto_compiler::{compile_with_language_version, LanguageVersion};
use atto_compiler::{lexer::lex, parser::Parser, ast::Stmt};

fn accepted(src: &str, version: LanguageVersion) {
    let result = compile_with_language_version(src, version)
        .unwrap_or_else(|e| panic!("expected compiled guest for {version:?}: {e}\n{src}"));
    assert_eq!(&result[..8], b"\0asm\x01\0\0\0");
    assert_eq!(result, compile_with_language_version(src, version).unwrap());
}

#[test]
fn old_unmarked_comma_tail_survives_v1_but_not_v2() {
    let src = "pub fnc main() -> int let int x = 5, x + 37 end";
    accepted(src, LanguageVersion::V1);
    let program = Parser::with_language_version(lex(src).unwrap(), LanguageVersion::V1)
        .parse_program().unwrap();
    assert!(matches!(program.funcs[0].body.last(), Some(Stmt::Return(Some(_), _))));
    let diagnostic = compile_with_language_version(src, LanguageVersion::V2).unwrap_err();
    assert_eq!(diagnostic.code, "E_LANGUAGE_VERSION");
    assert!(diagnostic.message.contains("::"), "{diagnostic}");
}

#[test]
fn explicit_double_colon_is_accepted_in_both_versions() {
    let src = "pub fnc main() -> int :: let int x = 5, x + 37 end";
    accepted(src, LanguageVersion::V1);
    accepted(src, LanguageVersion::V2);
    let legacy = compile_with_language_version(src, LanguageVersion::V1).unwrap();
    let strict = compile_with_language_version(src, LanguageVersion::V2).unwrap();
    assert_eq!(legacy, strict, "same checked codegen for shared syntax");
}

#[test]
fn strict_unmarked_statements_need_explicit_return() {
    let src = "pub fnc main() -> int const x: int = 5; ^ x + 37; end";
    accepted(src, LanguageVersion::V2);
    assert!(compile_with_language_version(src, LanguageVersion::V1).is_err());
}

#[test]
fn version_gate_covers_routines_and_inline_methods() {
    let routine = "routine answer() -> int let int x = 4, x + 38 end pub fnc main(): int { ^ answer(); }";
    accepted(routine, LanguageVersion::V1);
    assert_eq!(compile_with_language_version(routine, LanguageVersion::V2)
        .unwrap_err().code, "E_LANGUAGE_VERSION");

    let method = r#"
      define class Calculator as
        pub answer() -> int let int x = 4, x + 38 end
      end
      pub fnc main(): int { val c = new Calculator(); ^ c.answer(); }
    "#;
    accepted(method, LanguageVersion::V1);
    assert_eq!(compile_with_language_version(method, LanguageVersion::V2)
        .unwrap_err().code, "E_LANGUAGE_VERSION");
}

#[test]
fn out_of_line_class_receivers_do_not_bypass_version_gate() {
    let src = r#"
      define class Calculator as
        final int seed: 40;
      end
      pub mtd (self: Calculator) answer() -> int
        let int x = self.seed,
        x + 2
      end
      pub fnc main(): int { val c = new Calculator(); ^ c.answer(); }
    "#;
    accepted(src, LanguageVersion::V1);
    let diagnostic = compile_with_language_version(src, LanguageVersion::V2).unwrap_err();
    assert_eq!(diagnostic.code, "E_LANGUAGE_VERSION", "{diagnostic}");
}

#[test]
fn nested_argument_commas_are_not_header_mode_separators() {
    let src = r#"
      fnc add(int a, int b): int { ^ a + b; }
      pub fnc main() -> int
        let int x = 5,
        add(x, 37)
      end
    "#;
    accepted(src, LanguageVersion::V1);
    assert_eq!(compile_with_language_version(src, LanguageVersion::V2)
        .unwrap_err().code, "E_LANGUAGE_VERSION");
}
