use atto_compiler::compile;
#[test]
fn explicit_arm_results_compile_to_guest_wasm() {
    for source in [
        "pub fnc main(): int { return true ? { return 42; } : { ^ 7; }; }",
        "pub fnc main(): int { return false ? 7 : { ^ 42; }; }",
        "pub fnc main(): int { return true ? { return false ? 7 : 42; } : 0; }",
    ] {
        let wasm = compile(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert_eq!(&wasm[..4], b"\0asm");
    }
}
#[test]
fn direct_brace_arms_fail_closed_without_explicit_result() {
    for expr in ["true ? {} : 7", "false ? 7 : {}", "true ? { status: 42 } : 7",
                 "true ? { 42 } : 7", "true ? { val x = 42; return x; } : 7"] {
        let source = format!("pub fnc main(): int {{ return {expr}; }}");
        let error = compile(&source).expect_err(&source);
        assert!(format!("{error:?}").contains("E_VALUE_ARM"), "{source}: {error:?}");
    }
}
