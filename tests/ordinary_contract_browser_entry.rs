use atto_compiler::{compile, compile_verified_strict};

fn source(open: &str, close: &str, args: &str) -> String {
    format!("fnc divide(int a, int b) -> int\n    requires b != 0{open}\n    return a / b;\n{close}\npub fnc main() -> int {{ return divide({args}); }}")
}

#[test]
fn ordinary_contract_delimiters_emit_equivalent_guests() {
    for args in ["84, 2", "-85, 2", "-9223372036854775808, -1"] {
        let baseline = compile(&source(" {", "}", args)).unwrap();
        assert!(baseline.starts_with(b"\0asm"));
        for (open, close) in [("\n{", "}"), ("\nbegin", "end"), (" ::", "end"), ("\n::", "end")] {
            assert_eq!(compile(&source(open, close, args)).unwrap(), baseline);
        }
    }
}

#[test]
fn ordinary_contract_errors_do_not_fall_back() {
    for (open, close) in [(" begin", "end"), (" ::", "}"), (" ::", ""), (" :: begin", "end")] {
        assert_eq!(compile(&source(open, close, "84, 2")).unwrap_err().code, "E_SYNTAX");
    }
    assert_eq!(compile(&source(" {", "}", "84, 0")).unwrap_err().code, "E_REFINEMENT_PRECONDITION");
    let bad_suffix = format!("{} unexpected", source(" {", "}", "84, 2"));
    assert!(compile(&bad_suffix).is_err());
    assert!(compile_verified_strict(&source(" {", "}", "84, 2")).is_err());
    assert!(compile("pub fnc main() -> int { return 42; }").is_ok());
}
