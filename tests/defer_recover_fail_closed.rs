//! Negative admission gate for not-yet-lowered source exception scopes.
//! The same fixtures are also consumed by the native/wasm32 conformance
//! harness; a parser acceptance is not proof of guest unwinding support.

const RECOVERY: &str = "pub fnc main(): int { return 42; }";

#[test]
fn unsupported_exception_scopes_do_not_produce_wasm_and_compiler_recovers() {
    let cases = [
        ("defer", "pub fnc main(): int { defer stdio.println(1); return 42; }"),
        ("recover", "pub fnc main(): int { recover (|err, c| -> { return 42; }); return 0; }"),
        ("raise", "pub fnc main(): int { raise Error(\"escalation\"); return 42; }"),
        ("try/catch", "pub fnc main(): int { try { throw Error(\"failure\"); } catch (Error err) { return 42; } return 0; }"),
    ];
    for (name, source) in cases {
        let diagnostic = atto_compiler::compile(source).expect_err(name);
        assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{name}: {diagnostic}");
        assert!(diagnostic.pos.line > 0 && diagnostic.pos.col > 0, "{name}");
        let recovered = atto_compiler::compile(RECOVERY).expect("compiler must recover after rejection");
        assert!(recovered.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(recovered, atto_compiler::compile(RECOVERY).unwrap());
    }
}
