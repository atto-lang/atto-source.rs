//! Exact syntax fixture for /play's resource scope preview.
//! This tests named import header grammar and fail-closed compilation.
//! It does NOT claim that using/timeout/await are executable.
use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;

const PREVIEW: &str = r#"
import span from pkg "core.telemetry";
import sleep from pkg "std.time";

pub suspends fnc main(): int {
  using span("playground.demo") as current_span {
    with timeout(2000) {
      current_span.set_attribute("feature", "import");
      await sleep(1);
      stdio.println("span retained over suspension");
    }
  }
  return 0;
}
"#;

#[test]
fn named_imports_of_scoped_resource_example_are_supported_by_target_headers() {
    let source = r#"
        import span from pkg "core.telemetry";
        import sleep from pkg "std.time";
        pub fnc main(): int { return 0; }
    "#;
    let headers = parse_target_declarations(&lex(source).expect("lex")).expect("headers");
    assert_eq!(headers.imports, ["span", "sleep"]);
}

#[test]
fn actual_scoped_resource_demo_is_not_a_false_positive_wasm_guest() {
    let error = atto_compiler::compile(PREVIEW)
        .expect_err("using + timeout + await require a real compiler and runtime");
    assert!(!error.code.is_empty());
}

#[test]
#[ignore = "Enable only with real package linking, suspend-safe using cleanup, timeout and await guest execution"]
fn future_positive_compiler_gate_for_exact_scoped_resource_demo() {
    let guest = atto_compiler::compile(PREVIEW).expect("real executable guest required");
    assert!(guest.starts_with(b"\\0asm"), "must emit WebAssembly bytes");
}
