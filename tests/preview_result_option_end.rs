//! Real compiler coverage of the exact pinned Result/Option end-body preview.
//! The actual Wasm32-compiler guest-output oracle is enforced in
//! tests/playground-preview-audit.mjs, not by merely accepting the parser.
use atto_compiler::compile;

fn compiled(source: &str) -> Vec<u8> {
    let wasm = compile(source).unwrap_or_else(|e| panic!("expected guest, got {e}"));
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(compile(source).unwrap(), wasm, "deterministic Wasm");
    wasm
}

#[test]
fn exact_website_result_option_end_source_compiles() {
    compiled(include_str!("playground_previews/result-option-end.atto"));
}

#[test]
fn explicit_semicolon_end_body_supports_nested_fi_and_result() {
    compiled(r#"
        fnc validate(int n) -> Result<int, string>
            if n <= 0 then
                return Err("negative");
            fi
            return Ok(n);
        end
        pub fnc main(): int {
            match validate(42) over
                on Ok(n) -> { return n; }
                on Err(_) -> { return 0; }
            end
        }
    "#);
}

#[test]
fn legacy_comma_implicit_tail_remains_supported() {
    compiled(r#"
        fnc five() -> int
            let int three = 3,
            three + 2
        end
        fnc other() -> int ::
            let int three = 3,
            three + 2
        end
        pub fnc main(): int { return five() + other() + 32; }
    "#);
}

#[test]
fn invalid_end_body_never_emits_an_artifact_and_compiler_recovers() {
    for source in [
        "fnc bad() -> int return \"no\"; end pub fnc main(): int { return bad(); }",
        "fnc bad() -> int return 1; pub fnc main(): int { return bad(); }",
        "fnc bad() -> Result<int,string> return Ok(\"wrong\"); end pub fnc main(): int { return 1; }",
        "fnc bad() -> int if true then return 1; end pub fnc main(): int { return bad(); }",
        "fnc bad() -> int return false; end pub fnc main(): int { return bad(); }",
    ] {
        let result = compile(source);
        assert!(result.is_err(), "invalid source must not be admitted: {source}");
        let error = result.unwrap_err();
        assert!(error.code.starts_with("E_"), "{error}");
        assert!(error.pos.line > 0 && error.pos.col > 0, "{error}");
    }
    compiled("pub fnc main(): int { return 42; }");
}
