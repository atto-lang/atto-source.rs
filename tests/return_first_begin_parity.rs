use oreslang_source_rs::frontend::{parse, Expr};

#[test]
fn all_four_return_first_scalar_spellings_lower_to_same_ast() {
    for (index, header) in [
        "int <- main()",
        "int• main()",
        "fnc int <- main()",
        "fnc int• main()",
    ].iter().enumerate() {
        let closer = ["end", "end.", "end;", "end"][index];
        let source = format!("{header}\nbegin\n  return 42;\n{closer}\n");
        let program = parse(&source).expect(&source);
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].name, "main");
        assert_eq!(program.functions[0].expression, Expr::Integer(42));
    }
}

#[test]
fn scalar_helpers_are_callable_through_return_first_declarations() {
    let source = "fnc int <- add(int a, int b)\nbegin\nreturn a + b;\nend;\n\
                  int• main()\nbegin\nreturn add(40, 2);\nend.\n";
    let parsed = parse(source).unwrap();
    assert_eq!(parsed.functions.len(), 2);
}

#[test]
fn malformed_or_unlowerable_return_first_declarations_fail_closed() {
    for source in [
        "int <- main() begin return 42; end",
        "int <- main()\nbegin return 42;\nend",
        "int <- main()\nbegin\nreturn 42;",
        "fnc int <- main() -> int\nbegin\nreturn 42;\nend",
        "int <- main()\nbegin\nreturn 42;\nend..",
        "int <- main()\nbegin\nreturn 42;\nend. junk",
        "string <- main()\nbegin\nreturn 42;\nend",
        "int < - main()\nbegin\nreturn 42;\nend",
        "define class C as\n  int <- foo()\n  begin\n    return 42;\n  end\nend",
    ] {
        assert!(parse(source).is_err(), "must reject {source:?}");
    }
}

#[test]
fn trivia_and_newline_boundaries_are_enforced() {
    let source = "int <- main()\r\nbegin // comment\r\n  return 42;\r\nend. // comment\r\n";
    assert!(parse(source).is_ok());
    assert!(parse("int <- main()\nbegin; // invalid\nreturn 42;\nend").is_err());
    assert!(parse("int <- main()\nbegin end").is_err());
    // Duplicate declarations are a MIR semantic error, not a lexer/parser
    // syntax error. Keep that separation so conformance tests exercise the
    // actual compiler admission boundary rather than a false parse expectation.
    let duplicate = parse("int <- main()\nbegin\nreturn 42;\nend;\nfnc main(): int {return 0;}").unwrap();
    let error = oreslang_source_rs::mir::lower(duplicate).unwrap_err();
    assert_eq!(error.class, "duplicate_declaration");
}
