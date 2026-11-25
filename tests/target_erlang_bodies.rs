use atto_compiler::target_syntax::parse_target_declarations;
use atto_compiler::{compile, lexer::lex};

fn parse(src: &str) -> Result<atto_compiler::target_syntax::TargetHeaders, atto_compiler::Diagnostic> {
    parse_target_declarations(&lex(src).unwrap())
}

#[test]
fn mtd_receivers_admit_all_five_body_spellings_as_syntax_only() {
    let src = r#"
        declare actor A;
        mtd (self: A) a(): int { return 5; }
        mtd (self: A) b(): int -> { return 5; }
        mtd (self: A) c() -> int { return 5; }
        mtd (self: A) d() -> int
            let int x = 3,
            x + 2
        end
        mtd (self: A) e() -> int ::
            let int x = 3,
            x + 2
        end
        routine helper() -> int :: 5 end
    "#;
    let p = parse(src).unwrap();
    assert_eq!(p.receivers.len(), 5);
    assert!(p.receivers.iter().all(|m| m.return_type == "int"));
    assert_eq!(
        compile(src).unwrap_err().code,
        "E_UNSUPPORTED",
        "syntax-only mtd is not guest Wasm execution"
    );
}

#[test]
fn first_token_of_bare_erl_method_still_gets_sealed_state_validation() {
    for body in ["self.count end", ":: self.count end", "self.count = 2 end"] {
        let src = format!("declare actor A; mtd (self: A) unsafe_read() -> int {body}");
        let d = parse(&src).unwrap_err();
        assert_eq!(d.code, "E_SEALED", "{src}: {d}");
    }
}

#[test]
fn mtd_erlang_syntax_fails_closed() {
    let cases = [
        ("declare actor A; mtd (self: A) a() -> int :: 5,", "E_EOF"),
        ("declare actor A; mtd (self: A) a() -> int :: 5, end", "E_SYNTAX"),
        ("declare actor A; mtd (self: A) a() -> int :: 5; end", "E_SYNTAX"),
        ("declare actor A; mtd (self: A) a(): int :: 5 end", "E_SYNTAX"),
        ("declare actor A; mtd (self: A) a(): int -> 5 end", "E_SYNTAX"),
        ("declare actor A; mtd (self: A) a() -> int :: if true then 5 end", "E_SYNTAX"),
        (
            "declare actor A; mtd (self: A) a() -> int :: match 1 over on 1 -> {return 5;} end",
            "E_UNSUPPORTED",
        ),
    ];
    for (src, code) in cases {
        let d = parse(src).unwrap_err();
        assert_eq!(d.code, code, "{src}: {d}");
    }
}
