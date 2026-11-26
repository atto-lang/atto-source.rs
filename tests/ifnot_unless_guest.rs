//! Target conditional grammar: exact Rust parser and compiled guest admission.
//! Does not claim a public browser compiler release or cross-backend parity.
use atto_compiler::{compile, lexer::lex, parser::Parser};

#[test]
fn negated_conditional_heads_are_reserved_and_have_all_three_legal_forms() {
    // Keep the website's complete combined example executable here before
    // it can ever be promoted into the public browser's runnable tier.
    let source = include_str!("fixtures/ifnot-unless-guard-play.atto");
    let guest = compile(source).expect("full playground example must lower to a genuine Wasm guest");
    assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    for body in [
        "do ifnot false then return 7; fi return 42;",
        "do unless true then return 7; else return 42; fi",
        "ifnot false { return 7; } return 42;",
        "unless true { return 7; } else { return 42; }",
        "return ifnot false then 7 else 42 fi;",
        "return unless true then 7 else 42 fi;",
        "return ifnot false then 7 elif true then 8 else 42 fi;",
        "return if true then 7 else 42 fi;",
        "do if true then return 7; fi return 42;",
        "if true { return 7; } return 42;",
        "do if true then if false { return 7; } fi return 42;",
        "do if true then unless false { return 42; } fi return 9;",
        "if false { return 7; } fi return 42;",
        "if true do return 42; fi return 9;",
        "if false do return 9; else return 42; fi",
        "ifnot false do return 42; fi return 9;",
        "ifnot true do return 9; fi return 42;",
        "unless false do return 42; else return 9; fi",
        "unless true do return 9; elif true do return 42; else return 7; fi",
        "if true do unless true { return 9; } fi return 42;",
    ] {
        let src = format!("pub fnc main(): int {{ {body} }}");
        Parser::new(lex(&src).unwrap()).parse_program().unwrap();
        let guest = compile(&src).unwrap_or_else(|e| panic!("{body}: {e}"));
        assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0], "{body}");
    }
}

#[test]
fn invalid_conditionals_fail_closed_before_any_guest() {
    // Rejection must not poison compiler state or change a later valid guest.
    let valid = "pub fnc main(): int { return ifnot true then 7 else 42 fi; }";
    let stable_guest = compile(valid).expect("baseline negated value conditional must lower");
    for (body, expected) in [
        ("return ifnot false then 7 fi;", "E_VALUE_ARM"),
        ("return unless true then 7 fi;", "E_VALUE_ARM"),
        ("return ifnot false then else 9 fi;", "E_VALUE_ARM"),
        ("return unless true then 7 else fi;", "E_VALUE_ARM"),
        ("ifnot false then return 7; fi return 42;", "E_SYNTAX"),
        ("unless false then return 7; fi return 42;", "E_SYNTAX"),
        ("return unless 5 then 7 else 9 fi;", "E_TYPE"),
        ("return ifnot true then 7 else false fi;", "E_TYPE"),
        ("do unless 5 then return 7; fi return 42;", "E_TYPE"),
        ("ifnot 3 { return 7; } return 42;", "E_TYPE"),
        ("return unless true then return 7 else 9 fi;", "E_SYNTAX"),
        ("do unless true { return 7; }", "E_SYNTAX"),
        ("do ifnot false { return 7; }", "E_SYNTAX"),
        ("ifnot false { return 7; } fi return 42;", "E_SYNTAX"),
    ] {
        let src = format!("pub fnc main(): int {{ {body} }}");
        let error = match compile(&src) {
            Err(error) => error,
            Ok(_) => panic!("unexpected executable guest: {body}"),
        };
        assert_eq!(error.code, expected, "{body}: {error}");
        assert_eq!(
            compile(valid).unwrap(),
            stable_guest,
            "rejected conditional must not contaminate subsequent Wasm: {body}"
        );
    }
}
